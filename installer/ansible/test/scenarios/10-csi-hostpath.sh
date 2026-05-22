#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

cat <<YAML | run_playbook control-plane site.yml
---
all:
  children:
    tugboat_control_plane:
      hosts:
        localhost:
          ansible_connection: local
          ansible_become: false
          tugboat_build_mode: prebuilt
          tugboat_prebuilt_bin_dir: ${TUGBOAT_ANSIBLE_TEST_PREBUILT_BIN_DIR}
          tugboat_cargo_manifest_dir: /workspace
          tugboat_secure: false
          tugboat_apiserver_listen: 0.0.0.0:8080
          tugboat_apiserver_advertise_url: http://control-plane:8080
          tugboat_etcd_listen: 127.0.0.1:2379
          tugboat_etcd_peer_listen: 127.0.0.1:2380
          tugboat_etcd_node_name: control-plane
          tugboat_etcd_advertise_client_url: https://127.0.0.1:2379
          tugboat_etcd_initial_advertise_peer_url: https://127.0.0.1:2380
          tugboat_etcd_initial_cluster: control-plane=https://127.0.0.1:2380
          tugboat_etcd_endpoints:
            - https://127.0.0.1:2379
          tugboat_csi_hostpath_node_id: control-plane
    tugboat_workers:
      hosts: {}
    tugboat_csi_hostpath:
      hosts:
        localhost:
YAML

compose exec -T control-plane bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet hostpath-provisioner.service
timeout 30 bash -c '"'"'until test -S /var/run/csi/csi.sock; do sleep 1; done'"'"'
grep -q "socket_path = \"/var/run/csi/csi.sock\"" /etc/tugboat/controller-manager/config.toml
systemctl restart tugboat-controller-manager.service
systemctl is-active --quiet tugboat-controller-manager.service
'

worker_inventory false http://control-plane:8080 |
    run_playbook worker site.yml

compose exec -T worker bash -lc '
set -Eeuo pipefail
grep -q "\"hostpath.csi.k8s.io\" = \"/var/run/csi/csi.sock\"" /etc/tugboat/agent/config.toml
'

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
    "metadata": {"namespace": "demo", "name": "ansible-hostpath"},
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
    pvc = request("GET", "/api/v1/namespaces/demo/persistentvolumeclaims/ansible-hostpath")
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
    raise SystemExit("PersistentVolume for PVC demo/ansible-hostpath was not provisioned and bound")

request("DELETE", "/api/v1/namespaces/demo/persistentvolumeclaims/ansible-hostpath")
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
'
