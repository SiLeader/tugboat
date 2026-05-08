#!/usr/bin/env bash
set -Eeuo pipefail

: "${TUGBOAT_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_TEST_COMPOSE_FILE}" -p "${TUGBOAT_TEST_PROJECT_NAME}" "$@"
}

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace
bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --listen 0.0.0.0:8080 \
    --etcd-listen 0.0.0.0:2379 \
    --etcd-advertise-client-url https://control-plane:2379 \
    --etcd-endpoint https://control-plane:2379

bash installer/systemd/bootstrap-flannel-etcd.sh \
    --backend vxlan \
    --subnet 10.244.0.0/16 \
    --flannel-etcd-endpoints https://control-plane:2379 \
    --flannel-etcd-ca /etc/tugboat/pki/etcd/ca.crt \
    --flannel-etcd-cert /etc/tugboat/pki/etcd/client.crt \
    --flannel-etcd-key /etc/tugboat/pki/etcd/client.key

bash installer/systemd/bootstrap-flannel-etcd.sh \
    --backend vxlan \
    --subnet 10.244.0.0/16 \
    --flannel-etcd-endpoints https://control-plane:2379 \
    --flannel-etcd-ca /etc/tugboat/pki/etcd/ca.crt \
    --flannel-etcd-cert /etc/tugboat/pki/etcd/client.crt \
    --flannel-etcd-key /etc/tugboat/pki/etcd/client.key

ETCDCTL_API=3 etcdctl \
    --endpoints=https://control-plane:2379 \
    --cacert=/etc/tugboat/pki/etcd/ca.crt \
    --cert=/etc/tugboat/pki/etcd/client.crt \
    --key=/etc/tugboat/pki/etcd/client.key \
    get /coreos.com/network/config --print-value-only > /tmp/flannel-config.json

jq -e '.Network == \"10.244.0.0/16\" and .Backend.Type == \"vxlan\"' /tmp/flannel-config.json >/dev/null
"

compose exec -T control-plane tar -C /etc/tugboat/pki -czf - etcd |
    compose exec -T worker bash -lc 'mkdir -p /etc/tugboat/pki && tar -C /etc/tugboat/pki -xzf -'

compose exec -T worker bash -lc "
set -Eeuo pipefail
cd /workspace
bash installer/systemd/install-worker.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --apiserver-url http://control-plane:8080 \
    --node-name worker \
    --runtime qemu \
    --flannel-mode vxlan \
    --flannel-etcd-endpoints https://control-plane:2379 \
    --flannel-etcd-ca /etc/tugboat/pki/etcd/ca.crt \
    --flannel-etcd-cert /etc/tugboat/pki/etcd/client.crt \
    --flannel-etcd-key /etc/tugboat/pki/etcd/client.key

systemctl is-active --quiet flanneld.service
timeout 60 bash -c 'until test -s /run/flannel/subnet.env && grep -q \"^FLANNEL_NETWORK=10.244.0.0/16$\" /run/flannel/subnet.env; do sleep 1; done'
systemctl is-active --quiet tugboat-agent.service
grep -q '^Requires=flanneld.service$' /etc/systemd/system/tugboat-agent.service
grep -q '^After=flanneld.service$' /etc/systemd/system/tugboat-agent.service
if grep -q 'FLANNEL_NETWORK=' /etc/tmpfiles.d/tugboat-flannel.conf; then
    echo 'dynamic flannel mode rendered a static subnet.env tmpfiles entry' >&2
    exit 1
fi
"

compose exec -T control-plane python3 - http://localhost:8080/api/v1/nodes worker <<'PY'
import json
import sys
import time
import urllib.request

url = sys.argv[1]
node_name = sys.argv[2]

def has_ready_flannel(node):
    status = node.get("status", {})
    for plugin in status.get("cniPlugins", []):
        if plugin.get("name") == "flannel" and plugin.get("ready") is True:
            return True
    return False

for _ in range(30):
    with urllib.request.urlopen(url, timeout=2) as response:
        payload = json.load(response)
    for node in payload.get("items", []):
        metadata = node.get("metadata", {})
        if metadata.get("name") == node_name and has_ready_flannel(node):
            sys.exit(0)
    time.sleep(2)

raise SystemExit(f"node {node_name!r} with ready flannel CNI was not observed")
PY
