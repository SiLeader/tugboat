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

if ! compose exec -T worker test -f /workspace/installer/systemd/install-worker.sh; then
    echo 'skip: installer/systemd/install-worker.sh is not implemented yet'
    exit 0
fi

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace

if bash installer/systemd/uninstall.sh >/tmp/tugboat-uninstall-usage.out 2>&1; then
    echo 'uninstall.sh without target unexpectedly succeeded' >&2
    exit 1
fi
grep -q 'Usage: uninstall.sh' /tmp/tugboat-uninstall-usage.out

bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --insecure \
    --listen 0.0.0.0:8080 \
    --etcd-listen 127.0.0.1:2379

bash installer/systemd/uninstall.sh --control-plane
for service in etcd tugboat-apiserver tugboat-scheduler tugboat-controller-manager; do
    if systemctl is-active --quiet \"\${service}.service\"; then
        echo \"\${service}.service is still active after uninstall\" >&2
        exit 1
    fi
    test ! -e \"/etc/systemd/system/\${service}.service\"
done
test -d /var/lib/tugboat-etcd
mkdir -p \
    /var/lib/tugboat-apiserver \
    /var/lib/tugboat-scheduler \
    /var/lib/tugboat-controller-manager

bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --insecure \
    --listen 0.0.0.0:8080 \
    --etcd-listen 127.0.0.1:2379

curl -sf http://localhost:8080/healthz >/dev/null
"

compose exec -T worker bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/install-worker.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --apiserver-url http://control-plane:8080 \
    --insecure \
    --node-name worker \
    --runtime qemu

bash installer/systemd/uninstall.sh --worker
if systemctl is-active --quiet tugboat-agent.service; then
    echo 'tugboat-agent.service is still active after uninstall' >&2
    exit 1
fi
test ! -e /etc/systemd/system/tugboat-agent.service
test -d /var/lib/tugboat-agent

bash installer/systemd/install-worker.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --apiserver-url http://control-plane:8080 \
    --insecure \
    --node-name worker \
    --runtime qemu
"

compose exec -T control-plane curl -sf http://localhost:8080/healthz >/dev/null

compose exec -T worker bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/uninstall.sh --worker --purge
test ! -e /var/lib/tugboat-agent
test ! -e /run/flannel
test ! -e /opt/cni/bin/bridge
test ! -e /opt/cni/bin/loopback
test ! -e /opt/cni/bin/flannel
test ! -e /opt/cni/bin/flanneld
test ! -e /etc/tugboat
"

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
cd /workspace

bash installer/systemd/uninstall.sh --control-plane --purge
test ! -e /var/lib/tugboat-etcd
test ! -e /var/lib/tugboat-apiserver
test ! -e /var/lib/tugboat-scheduler
test ! -e /var/lib/tugboat-controller-manager
test ! -e /etc/tugboat
"
