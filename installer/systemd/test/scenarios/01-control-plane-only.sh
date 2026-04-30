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

if [[ ! -f installer/systemd/install-control-plane.sh ]]; then
    echo 'skip: installer/systemd/install-control-plane.sh is not implemented yet'
    exit 0
fi

bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --listen 0.0.0.0:8080 \
    --etcd-listen 127.0.0.1:2379

systemctl is-active --quiet etcd.service
systemctl is-active --quiet tugboat-apiserver.service
systemctl is-active --quiet tugboat-scheduler.service
systemctl is-active --quiet tugboat-controller-manager.service

curl -sf http://localhost:8080/healthz >/dev/null
curl -sf http://localhost:8080/api/v1/nodes | python3 -m json.tool >/dev/null

if [[ -f installer/systemd/uninstall.sh ]]; then
    bash installer/systemd/uninstall.sh
fi
"
