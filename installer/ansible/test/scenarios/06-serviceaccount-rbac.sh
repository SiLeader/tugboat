#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

control_plane_inventory true 0.0.0.0:8443 https://control-plane:8443 127.0.0.1:2379 https://127.0.0.1:2379 |
    run_playbook control-plane site.yml

# shellcheck disable=SC2016
compose exec -T control-plane bash -lc '
set -Eeuo pipefail
grep -q "mode = \"RBAC\"" /etc/tugboat/apiserver/config.toml
grep -q "anonymous_enabled = false" /etc/tugboat/apiserver/config.toml
grep -q "type = \"service-account\"" /etc/tugboat/scheduler/config.toml
grep -q "type = \"service-account\"" /etc/tugboat/controller-manager/config.toml
test -s /var/run/secrets/tugboat.cloud/serviceaccount/scheduler/token
test -s /var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token
test -s /var/run/secrets/tugboat.cloud/serviceaccount/agent/token

for token in \
    /var/run/secrets/tugboat.cloud/serviceaccount/scheduler/token \
    /var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token \
    /var/run/secrets/tugboat.cloud/serviceaccount/agent/token; do
    mode="$(stat -c "%a" "${token}")"
    case "${mode}" in
        *[4-7])
            echo "token is world-readable: ${token} mode=${mode}" >&2
            exit 1
            ;;
    esac
done

curl_status="$(curl -sk -o /tmp/tugboat-anonymous-nodes.json -w "%{http_code}" \
    --cacert /etc/tugboat/pki/ca.crt \
    https://localhost:8443/api/v1/nodes)"
case "${curl_status}" in
    401|403) ;;
    *)
        echo "anonymous GET /api/v1/nodes returned ${curl_status}, expected 401 or 403" >&2
        cat /tmp/tugboat-anonymous-nodes.json >&2
        exit 1
        ;;
esac
'

compose exec -T control-plane cat /etc/tugboat/pki/ca.crt |
    compose exec -T worker tee /tmp/tugboat-ca.crt >/dev/null
compose exec -T control-plane cat /var/run/secrets/tugboat.cloud/serviceaccount/agent/token |
    compose exec -T worker tee /tmp/tugboat-agent-token >/dev/null

worker_inventory true https://control-plane:8443 '
          tugboat_ca_cert: /tmp/tugboat-ca.crt
          tugboat_worker_ca_dest_path: /tmp/tugboat-ca.crt
          tugboat_agent_service_account_token: /tmp/tugboat-agent-token
          tugboat_worker_service_account_token_dest_path: /tmp/tugboat-agent-token
          tugboat_worker_distribute_secure_material: false' |
    run_playbook worker site.yml

# shellcheck disable=SC2016
compose exec -T worker bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet tugboat-agent.service
grep -q "type = \"service-account\"" /etc/tugboat/agent/config.toml
test -s /var/run/secrets/tugboat.cloud/serviceaccount/agent/token
mode="$(stat -c "%a" /var/run/secrets/tugboat.cloud/serviceaccount/agent/token)"
case "${mode}" in
    *[4-7])
        echo "agent token is world-readable: mode=${mode}" >&2
        exit 1
        ;;
esac
'

compose exec -T control-plane python3 - https://localhost:8443/api/v1/nodes worker /etc/tugboat/pki/ca.crt /var/run/secrets/tugboat.cloud/serviceaccount/agent/token <<'PY'
import json
import ssl
import sys
import time
import urllib.request

url = sys.argv[1]
node_name = sys.argv[2]
ca_cert = sys.argv[3]
token_path = sys.argv[4]
context = ssl.create_default_context(cafile=ca_cert)
with open(token_path, encoding="utf-8") as f:
    token = f.read().strip()
headers = {"Authorization": f"Bearer {token}"}

for _ in range(30):
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.urlopen(request, timeout=2, context=context) as response:
        payload = json.load(response)
    for node in payload.get("items", []):
        metadata = node.get("metadata", {})
        if metadata.get("name") == node_name:
            sys.exit(0)
    time.sleep(2)

raise SystemExit(f"node {node_name!r} was not observed with ServiceAccount auth")
PY
