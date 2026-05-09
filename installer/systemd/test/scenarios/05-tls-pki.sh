#!/usr/bin/env bash
set -Eeuo pipefail

: "${TUGBOAT_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_TEST_COMPOSE_FILE}" -p "${TUGBOAT_TEST_PROJECT_NAME}" "$@"
}

if ! compose exec -T control-plane test -f /workspace/installer/systemd/setup-pki.sh; then
    echo 'skip: installer/systemd/setup-pki.sh is not implemented yet'
    exit 0
fi

if ! compose exec -T control-plane test -f /workspace/installer/systemd/install-control-plane.sh; then
    echo 'skip: installer/systemd/install-control-plane.sh is not implemented yet'
    exit 0
fi

if ! compose exec -T worker test -f /workspace/installer/systemd/install-worker.sh; then
    echo 'skip: installer/systemd/install-worker.sh is not implemented yet'
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

systemctl is-active --quiet etcd.service
systemctl is-active --quiet tugboat-apiserver.service
systemctl is-active --quiet tugboat-scheduler.service
systemctl is-active --quiet tugboat-controller-manager.service

curl -sf --cacert /etc/tugboat/pki/ca.crt https://localhost:8443/healthz >/dev/null

openssl x509 -in /etc/tugboat/pki/apiserver.crt -noout -ext subjectAltName > /tmp/tugboat-san.txt
grep -q 'DNS:localhost' /tmp/tugboat-san.txt
grep -q 'DNS:control-plane' /tmp/tugboat-san.txt
grep -q 'IP Address:127.0.0.1' /tmp/tugboat-san.txt

grep -q 'cert_file = \"/etc/tugboat/pki/apiserver.crt\"' /etc/tugboat/apiserver/config.toml
grep -q 'key_file = \"/etc/tugboat/pki/apiserver.key\"' /etc/tugboat/apiserver/config.toml
grep -q 'ca_cert_path = \"/etc/tugboat/pki/ca.crt\"' /etc/tugboat/scheduler/config.toml
grep -q 'ca_cert_path = \"/etc/tugboat/pki/ca.crt\"' /etc/tugboat/controller-manager/config.toml

before=\"\$(sha256sum /etc/tugboat/pki/ca.crt /etc/tugboat/pki/apiserver.crt)\"
bash installer/systemd/setup-pki.sh \
    --pki-dir /etc/tugboat/pki \
    --apiserver-host control-plane \
    --apiserver-ip 127.0.0.1
after=\"\$(sha256sum /etc/tugboat/pki/ca.crt /etc/tugboat/pki/apiserver.crt)\"
if [[ \"\${before}\" != \"\${after}\" ]]; then
    echo 'setup-pki.sh changed existing certificates without --force' >&2
    exit 1
fi
"

compose exec -T control-plane cat /etc/tugboat/pki/ca.crt |
    compose exec -T worker tee /tmp/tugboat-ca.crt >/dev/null
compose exec -T control-plane cat /var/run/secrets/tugboat.cloud/serviceaccount/agent/token |
    compose exec -T worker tee /tmp/tugboat-agent-token >/dev/null

compose exec -T worker bash -lc "
set -Eeuo pipefail
cd /workspace

if bash installer/systemd/install-worker.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --apiserver-url https://control-plane:8443 \
    --node-name worker \
    --runtime qemu >/tmp/tugboat-worker-no-ca.out 2>&1; then
    echo 'install-worker.sh accepted an https apiserver URL without --ca-cert' >&2
    exit 1
fi
grep -q -- '--ca-cert is required' /tmp/tugboat-worker-no-ca.out

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
grep -q 'ca_cert_path = \"/etc/tugboat/pki/ca.crt\"' /etc/tugboat/agent/config.toml
grep -q 'type = \"service-account\"' /etc/tugboat/agent/config.toml
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

raise SystemExit(f"node {node_name!r} was not observed over HTTPS")
PY
