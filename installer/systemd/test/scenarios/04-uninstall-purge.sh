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

if ! compose exec -T control-plane test -f /workspace/installer/systemd/uninstall.sh; then
    echo 'skip: installer/systemd/uninstall.sh is not implemented yet'
    exit 0
fi

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --listen 0.0.0.0:8080 \
    --etcd-listen 127.0.0.1:2379

bash installer/systemd/uninstall.sh
test -d /var/lib/tugboat-etcd

bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --listen 0.0.0.0:8080 \
    --etcd-listen 127.0.0.1:2379

bash installer/systemd/uninstall.sh --purge
test ! -e /var/lib/tugboat-etcd
test ! -e /etc/tugboat
"
