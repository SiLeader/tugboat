#!/usr/bin/env bash
set -Eeuo pipefail

: "${TUGBOAT_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_TEST_COMPOSE_FILE}" -p "${TUGBOAT_TEST_PROJECT_NAME}" "$@"
}

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
if ! curl -sf http://localhost:8080/healthz >/dev/null; then
    bash installer/systemd/install-control-plane.sh \
        --use-prebuilt \
        --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
        --insecure \
        --listen 0.0.0.0:8080 \
        --etcd-listen 127.0.0.1:2379
fi
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

systemctl is-active --quiet tugboat-agent.service
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
    plugins = status.get("cniPlugins", [])
    for plugin in plugins:
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
