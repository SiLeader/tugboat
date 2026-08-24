#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

control_plane_inventory false 0.0.0.0:8080 http://control-plane:8080 127.0.0.1:2379 https://127.0.0.1:2379 |
    run_playbook control-plane site.yml

# shellcheck disable=SC2016
compose exec -T control-plane bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet etcd.service
systemctl is-active --quiet tugboat-apiserver.service
systemctl is-active --quiet tugboat-scheduler.service
systemctl is-active --quiet tugboat-controller-manager.service
curl -sf http://localhost:8080/healthz >/dev/null
nodes_json="$(curl -sf http://localhost:8080/api/v1/nodes)"
python3 - <<'"'"'PY'"'"' "${nodes_json}"
import json
import sys

payload = json.loads(sys.argv[1])
if payload.get("items") != []:
    raise SystemExit(f"expected empty node list, got: {payload!r}")
PY
'
