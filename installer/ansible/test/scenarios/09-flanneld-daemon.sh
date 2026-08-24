#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

control_plane_inventory false 0.0.0.0:8080 http://control-plane:8080 0.0.0.0:2379 https://control-plane:2379 '
          tugboat_flannel_mode: vxlan
          tugboat_flannel_etcd_endpoints: https://control-plane:2379' |
    run_playbook control-plane site.yml

compose exec -T control-plane bash -lc '
set -Eeuo pipefail
ETCDCTL_API=3 etcdctl \
    --endpoints=https://control-plane:2379 \
    --cacert=/etc/tugboat/pki/etcd/ca.crt \
    --cert=/etc/tugboat/pki/etcd/client.crt \
    --key=/etc/tugboat/pki/etcd/client.key \
    get /coreos.com/network/config --print-value-only > /tmp/flannel-config.json
jq -e ".Network == \"10.244.0.0/16\" and .Backend.Type == \"vxlan\"" /tmp/flannel-config.json >/dev/null
'

compose exec -T control-plane tar -C /etc/tugboat/pki -czf - etcd |
    compose exec -T worker bash -lc 'mkdir -p /etc/tugboat/pki && tar -C /etc/tugboat/pki -xzf -'

worker_inventory false http://control-plane:8080 '
          tugboat_flannel_mode: vxlan
          tugboat_flannel_etcd_endpoints: https://control-plane:2379
          tugboat_worker_distribute_secure_material: false' |
    run_playbook worker site.yml

compose exec -T worker bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet flanneld.service
timeout 60 bash -c '"'"'until test -s /run/flannel/subnet.env && grep -q "^FLANNEL_NETWORK=10.244.0.0/16$" /run/flannel/subnet.env; do sleep 1; done'"'"'
systemctl is-active --quiet tugboat-agent.service
grep -q "^Requires=flanneld.service$" /etc/systemd/system/tugboat-agent.service
grep -q "^After=flanneld.service$" /etc/systemd/system/tugboat-agent.service
if grep -q "FLANNEL_NETWORK=" /etc/tmpfiles.d/tugboat-flannel.conf; then
    echo "dynamic flannel mode rendered a static subnet.env tmpfiles entry" >&2
    exit 1
fi
'

compose exec -T control-plane python3 - http://localhost:8080/api/v1/nodes worker <<'PY'
import json
import sys
import time
import urllib.request

url = sys.argv[1]
node_name = sys.argv[2]

def has_ready_flannel(node):
    status = node.get("status", {})
    return any(
        plugin.get("name") == "flannel" and plugin.get("ready") is True
        for plugin in status.get("cniPlugins", [])
    )

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
