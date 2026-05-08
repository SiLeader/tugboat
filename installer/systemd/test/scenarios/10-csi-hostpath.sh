#!/usr/bin/env bash
set -Eeuo pipefail

: "${TUGBOAT_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_TEST_COMPOSE_FILE}" -p "${TUGBOAT_TEST_PROJECT_NAME}" "$@"
}

if ! compose exec -T control-plane test -f /workspace/installer/systemd/install-csi-hostpath.sh; then
    echo 'skip: installer/systemd/install-csi-hostpath.sh is not implemented yet'
    exit 0
fi

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace

if ! curl -sf http://localhost:8080/healthz >/dev/null; then
    bash installer/systemd/install-control-plane.sh \
        --use-prebuilt \
        --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
        --listen 0.0.0.0:8080 \
        --etcd-listen 127.0.0.1:2379
fi

bash installer/systemd/install-csi-hostpath.sh --build --node-id control-plane
systemctl is-active --quiet hostpath-provisioner.service
timeout 30 bash -c 'until test -S /var/run/csi/csi.sock; do sleep 1; done'
grep -q 'socket_path = \"/var/run/csi/csi.sock\"' /etc/tugboat/controller-manager/config.toml

systemctl restart tugboat-controller-manager.service
systemctl is-active --quiet tugboat-controller-manager.service
"

compose exec -T worker bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/install-worker.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --apiserver-url http://control-plane:8080 \
    --node-name worker \
    --runtime qemu

grep -q '\"hostpath.csi.k8s.io\" = \"/var/run/csi/csi.sock\"' /etc/tugboat/agent/config.toml
"

compose exec -T control-plane python3 - <<'PY'
import json
import time
import urllib.error
import urllib.request

BASE = "http://localhost:8080"

def request(method, path, payload=None, ok=(200, 201)):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload).encode()
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=5) as response:
            body = response.read()
            if response.status not in ok:
                raise SystemExit(f"{method} {path} returned {response.status}: {body!r}")
            return json.loads(body or b"{}")
    except urllib.error.HTTPError as exc:
        if exc.code in ok:
            body = exc.read()
            return json.loads(body or b"{}")
        raise

def create(path, payload):
    try:
        return request("POST", path, payload, ok=(200, 201))
    except urllib.error.HTTPError as exc:
        if exc.code == 409:
            return {}
        raise

create("/api/v1/namespaces", {
    "apiVersion": "v1",
    "kind": "Namespace",
    "metadata": {"name": "demo"},
})
create("/api/v1/storageclasses", {
    "apiVersion": "v1",
    "kind": "StorageClass",
    "metadata": {"name": "hostpath"},
    "spec": {
        "provisioner": "hostpath.csi.k8s.io",
        "reclaimPolicy": "Delete",
        "allowVolumeExpansion": True,
    },
})
create("/api/v1/namespaces/demo/persistentvolumeclaims", {
    "apiVersion": "v1",
    "kind": "PersistentVolumeClaim",
    "metadata": {"namespace": "demo", "name": "systemd-hostpath"},
    "spec": {
        "accessModes": ["ReadWriteOnce"],
        "storageClassName": "hostpath",
        "volumeMode": "Filesystem",
        "requestedCapacityBytes": 1048576,
    },
})

volume_name = None
handle = None
for _ in range(60):
    pvc = request("GET", "/api/v1/namespaces/demo/persistentvolumeclaims/systemd-hostpath")
    spec = pvc.get("spec") or {}
    status = pvc.get("status") or {}
    bound_volume_name = (
        spec.get("volume_name")
        or spec.get("volumeName")
        or status.get("volume_name")
        or status.get("volumeName")
    )
    if bound_volume_name:
        pv = request("GET", f"/api/v1/persistentvolumes/{bound_volume_name}")
        pv_status = pv.get("status") or {}
        source = ((pv.get("spec") or {}).get("csi") or {})
        bound_handle = source.get("volume_handle") or source.get("volumeHandle")
        if pv_status.get("phase") == "Bound" and bound_handle:
            volume_name = bound_volume_name
            handle = bound_handle
            break
    if status.get("phase") == "Bound" and bound_volume_name:
        volume_name = bound_volume_name
        break
    time.sleep(2)

if not volume_name or not handle:
    raise SystemExit("PersistentVolume for PVC demo/systemd-hostpath was not provisioned and bound")

request("DELETE", "/api/v1/namespaces/demo/persistentvolumeclaims/systemd-hostpath")
request("DELETE", f"/api/v1/persistentvolumes/{volume_name}")

with open("/tmp/tugboat-csi-volume-handle", "w", encoding="utf-8") as fh:
    fh.write(handle)
PY

# shellcheck disable=SC2016
compose exec -T control-plane bash -lc '
set -Eeuo pipefail
handle="$(cat /tmp/tugboat-csi-volume-handle)"
for _ in $(seq 1 60); do
    if ! find /var/lib/tugboat-csi-hostpath -mindepth 1 -maxdepth 2 -name "${handle}" | grep -q .; then
        break
    fi
    sleep 2
done
if find /var/lib/tugboat-csi-hostpath -mindepth 1 -maxdepth 2 -name "${handle}" | grep -q .; then
    echo "hostpath volume ${handle} still exists after PV deletion" >&2
    exit 1
fi

bash installer/systemd/uninstall.sh --control-plane --purge
test ! -e /etc/systemd/system/hostpath-provisioner.service
test ! -e /usr/local/bin/hostpathplugin
test ! -e /var/run/csi
test ! -e /var/lib/tugboat-csi-hostpath
'
