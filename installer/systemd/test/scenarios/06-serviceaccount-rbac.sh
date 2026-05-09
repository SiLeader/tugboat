#!/usr/bin/env bash
set -Eeuo pipefail

: "${TUGBOAT_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_TEST_COMPOSE_FILE}" -p "${TUGBOAT_TEST_PROJECT_NAME}" "$@"
}

if ! compose exec -T control-plane test -f /workspace/installer/systemd/bootstrap-rbac.sh; then
    echo 'skip: installer/systemd/bootstrap-rbac.sh is not implemented yet'
    exit 0
fi

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --secure \
    --etcd-listen 127.0.0.1:2379

systemctl is-active --quiet tugboat-apiserver.service
systemctl is-active --quiet tugboat-scheduler.service
systemctl is-active --quiet tugboat-controller-manager.service

grep -q 'mode = \"RBAC\"' /etc/tugboat/apiserver/config.toml
grep -q 'anonymous_enabled = false' /etc/tugboat/apiserver/config.toml
grep -q 'type = \"service-account\"' /etc/tugboat/scheduler/config.toml
grep -q 'type = \"service-account\"' /etc/tugboat/controller-manager/config.toml
grep -q '/var/run/secrets/tugboat.cloud/serviceaccount/scheduler/token' /etc/tugboat/scheduler/config.toml
grep -q '/var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token' /etc/tugboat/controller-manager/config.toml

test -s /var/run/secrets/tugboat.cloud/serviceaccount/scheduler/token
test -s /var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token
test -s /var/run/secrets/tugboat.cloud/serviceaccount/agent/token

for token in \
    /var/run/secrets/tugboat.cloud/serviceaccount/scheduler/token \
    /var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token \
    /var/run/secrets/tugboat.cloud/serviceaccount/agent/token; do
    mode=\"\$(stat -c '%a' \"\${token}\")\"
    case \"\${mode}\" in
        *[4-7])
            echo \"token is world-readable: \${token} mode=\${mode}\" >&2
            exit 1
            ;;
    esac
done

curl_status=\"\$(curl -sk -o /tmp/tugboat-anonymous-nodes.json -w '%{http_code}' \
    --cacert /etc/tugboat/pki/ca.crt \
    https://localhost:8443/api/v1/nodes)\"
case \"\${curl_status}\" in
    401|403) ;;
    *)
        echo \"anonymous GET /api/v1/nodes returned \${curl_status}, expected 401 or 403\" >&2
        cat /tmp/tugboat-anonymous-nodes.json >&2
        exit 1
        ;;
esac

bash installer/systemd/bootstrap-rbac.sh \
    --apiserver-url https://localhost:8443 \
    --ca-cert /etc/tugboat/pki/ca.crt \
    --token-output-root /var/run/secrets/tugboat.cloud/serviceaccount \
    --token-owner-group tugboat
"

compose exec -T control-plane cat /etc/tugboat/pki/ca.crt |
    compose exec -T worker tee /tmp/tugboat-ca.crt >/dev/null
compose exec -T control-plane cat /var/run/secrets/tugboat.cloud/serviceaccount/agent/token |
    compose exec -T worker tee /tmp/tugboat-agent-token >/dev/null

compose exec -T worker bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/install-worker.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --apiserver-url https://control-plane:8443 \
    --ca-cert /tmp/tugboat-ca.crt \
    --secure \
    --service-account-token /tmp/tugboat-agent-token \
    --node-name worker \
    --runtime qemu

systemctl is-active --quiet tugboat-agent.service
grep -q 'type = \"service-account\"' /etc/tugboat/agent/config.toml
test -s /var/run/secrets/tugboat.cloud/serviceaccount/agent/token
mode=\"\$(stat -c '%a' /var/run/secrets/tugboat.cloud/serviceaccount/agent/token)\"
case \"\${mode}\" in
    *[4-7])
        echo \"agent token is world-readable: mode=\${mode}\" >&2
        exit 1
        ;;
esac
"

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
