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

if ! compose exec -T control-plane test -f /workspace/installer/systemd/bootstrap-flannel.sh; then
    echo 'skip: installer/systemd/bootstrap-flannel.sh is not implemented yet'
    exit 0
fi

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace
if ! curl -sf http://localhost:8080/healthz >/dev/null; then
    bash installer/systemd/install-control-plane.sh \
        --use-prebuilt \
        --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
        --listen 0.0.0.0:8080 \
        --etcd-listen 127.0.0.1:2379
fi

bash installer/systemd/bootstrap-flannel.sh
bash installer/systemd/bootstrap-flannel.sh
"

compose exec -T control-plane python3 - http://localhost:8080/api/v1/clusternetworkclasses/cluster-network <<'PY'
import json
import sys
import urllib.request

with urllib.request.urlopen(sys.argv[1], timeout=2) as response:
    payload = json.load(response)

spec = payload.get("spec", {})
if spec.get("cniPlugin") != "flannel":
    raise SystemExit("cluster-network cniPlugin is not flannel")
PY
