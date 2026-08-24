#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

control_plane_inventory true 0.0.0.0:8443 https://control-plane:8443 127.0.0.1:2379 https://127.0.0.1:2379 |
    run_playbook control-plane site.yml

compose exec -T control-plane bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet etcd.service
systemctl is-active --quiet tugboat-apiserver.service
systemctl is-active --quiet tugboat-scheduler.service
systemctl is-active --quiet tugboat-controller-manager.service
curl -sf --cacert /etc/tugboat/pki/ca.crt https://localhost:8443/healthz >/dev/null

openssl x509 -in /etc/tugboat/pki/apiserver.crt -noout -ext subjectAltName > /tmp/tugboat-san.txt
grep -q "DNS:localhost" /tmp/tugboat-san.txt
grep -q "DNS:control-plane" /tmp/tugboat-san.txt
grep -q "IP Address:127.0.0.1" /tmp/tugboat-san.txt

grep -q "cert_file = \"/etc/tugboat/pki/apiserver.crt\"" /etc/tugboat/apiserver/config.toml
grep -q "key_file = \"/etc/tugboat/pki/apiserver.key\"" /etc/tugboat/apiserver/config.toml
grep -q "ca_cert_path = \"/etc/tugboat/pki/ca.crt\"" /etc/tugboat/scheduler/config.toml
grep -q "ca_cert_path = \"/etc/tugboat/pki/ca.crt\"" /etc/tugboat/controller-manager/config.toml
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

compose exec -T worker bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet tugboat-agent.service
grep -q "ca_cert_path = \"/etc/tugboat/pki/ca.crt\"" /etc/tugboat/agent/config.toml
grep -q "type = \"service-account\"" /etc/tugboat/agent/config.toml
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

raise SystemExit(f"node {node_name!r} was not observed over HTTPS")
PY
