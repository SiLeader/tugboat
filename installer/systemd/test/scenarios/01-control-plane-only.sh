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
    --insecure \
    --listen 0.0.0.0:8080 \
    --etcd-listen 127.0.0.1:2379

systemctl is-active --quiet etcd.service
systemctl is-active --quiet tugboat-apiserver.service
systemctl is-active --quiet tugboat-scheduler.service
systemctl is-active --quiet tugboat-controller-manager.service

if ! verify_output=\"\$(systemd-analyze verify \
    /etc/systemd/system/etcd.service \
    /etc/systemd/system/tugboat-apiserver.service \
    /etc/systemd/system/tugboat-scheduler.service \
    /etc/systemd/system/tugboat-controller-manager.service 2>&1)\"; then
    printf '%s\n' \"\${verify_output}\" >&2
    exit 1
fi
if [[ -n \"\${verify_output}\" ]]; then
    printf '%s\n' \"\${verify_output}\" >&2
    exit 1
fi

old_pid=\"\$(systemctl show -P MainPID tugboat-scheduler.service)\"
if [[ -z \"\${old_pid}\" || \"\${old_pid}\" == 0 ]]; then
    echo 'tugboat-scheduler.service has no running MainPID' >&2
    exit 1
fi

kill -9 \"\${old_pid}\"
timeout 30 bash -c '
    set -Eeuo pipefail
    old_pid=\"\$1\"
    while true; do
        new_pid=\"\$(systemctl show -P MainPID tugboat-scheduler.service)\"
        if [[ \"\${new_pid}\" != 0 && \"\${new_pid}\" != \"\${old_pid}\" ]] &&
           systemctl is-active --quiet tugboat-scheduler.service; then
            exit 0
        fi
        sleep 1
    done
' _ \"\${old_pid}\"

curl -sf http://localhost:8080/healthz >/dev/null
nodes_json=\"\$(curl -sf http://localhost:8080/api/v1/nodes)\"
python3 - <<'PY' \"\${nodes_json}\"
import json
import sys

payload = json.loads(sys.argv[1])
if payload.get(\"items\") != []:
    raise SystemExit(f\"expected empty node list, got: {payload!r}\")
PY

if [[ -f installer/systemd/uninstall.sh ]]; then
    bash installer/systemd/uninstall.sh --control-plane
fi
"
