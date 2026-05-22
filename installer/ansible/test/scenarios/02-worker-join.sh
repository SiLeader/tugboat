#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

control_plane_inventory false 0.0.0.0:8080 http://control-plane:8080 127.0.0.1:2379 https://127.0.0.1:2379 |
    run_playbook control-plane site.yml

worker_inventory false http://control-plane:8080 |
    run_playbook worker site.yml

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
