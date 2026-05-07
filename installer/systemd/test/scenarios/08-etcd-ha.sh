#!/usr/bin/env bash
set -Eeuo pipefail

: "${TUGBOAT_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_TEST_COMPOSE_FILE}" -p "${TUGBOAT_TEST_PROJECT_NAME}" "$@"
}

wait_for_systemd() {
    local service="$1"

    compose exec -T "${service}" bash -lc "timeout 60 bash -c 'until state=\$(systemctl is-system-running); [[ \"\${state}\" == running || \"\${state}\" == degraded ]]; do sleep 1; done'"
}

install_control_plane() {
    local service="$1"
    local name="$2"

    compose exec -T "${service}" bash -lc "
set -Eeuo pipefail
cd /workspace
bash installer/systemd/install-control-plane.sh \
    --use-prebuilt \
    --bin-dir '${TUGBOAT_TEST_PREBUILT_BIN_DIR}' \
    --listen 0.0.0.0:8080 \
    --etcd-listen 0.0.0.0:2379 \
    --etcd-peer-listen 0.0.0.0:2380 \
    --etcd-node-name '${name}' \
    --etcd-advertise-client-url 'https://${service}:2379' \
    --etcd-initial-advertise-peer-url 'https://${service}:2380' \
    --etcd-initial-cluster 'cp1=https://control-plane:2380,cp2=https://control-plane-2:2380,cp3=https://control-plane-3:2380' \
    --etcd-endpoint 'https://control-plane:2379' \
    --etcd-endpoint 'https://control-plane-2:2379' \
    --etcd-endpoint 'https://control-plane-3:2379'
"
}

compose up -d control-plane-2 control-plane-3
wait_for_systemd control-plane-2
wait_for_systemd control-plane-3

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
apt-get update
apt-get install -y openssl
mkdir -p /etc/tugboat/pki
bash /workspace/installer/systemd/setup-etcd-pki.sh \
    --pki-dir /etc/tugboat/pki/etcd \
    --server-host control-plane \
    --server-host control-plane-2 \
    --server-host control-plane-3 \
    --peer-host control-plane \
    --peer-host control-plane-2 \
    --peer-host control-plane-3
"

compose exec -T control-plane tar -C /etc/tugboat/pki -czf - etcd |
    compose exec -T control-plane-2 bash -lc 'mkdir -p /etc/tugboat/pki && tar -C /etc/tugboat/pki -xzf -'
compose exec -T control-plane tar -C /etc/tugboat/pki -czf - etcd |
    compose exec -T control-plane-3 bash -lc 'mkdir -p /etc/tugboat/pki && tar -C /etc/tugboat/pki -xzf -'

pids=()
install_control_plane control-plane cp1 &
pids+=("$!")
install_control_plane control-plane-2 cp2 &
pids+=("$!")
install_control_plane control-plane-3 cp3 &
pids+=("$!")

for pid in "${pids[@]}"; do
    wait "${pid}"
done

compose exec -T control-plane bash -lc "
set -Eeuo pipefail

for service in etcd tugboat-apiserver; do
    systemctl is-active --quiet \"\${service}.service\"
done

ETCDCTL_API=3 etcdctl \
    --endpoints=https://control-plane:2379,https://control-plane-2:2379,https://control-plane-3:2379 \
    --cacert=/etc/tugboat/pki/etcd/ca.crt \
    --cert=/etc/tugboat/pki/etcd/client.crt \
    --key=/etc/tugboat/pki/etcd/client.key \
    endpoint status -w json >/tmp/etcd-status.json

jq -e 'length == 3 and any(.[]; .Status.leader != 0)' /tmp/etcd-status.json >/dev/null
grep -q '\"https://control-plane-2:2379\"' /etc/tugboat/apiserver/config.toml
curl -sf http://localhost:8080/healthz >/dev/null
curl -sf http://localhost:8080/api/v1/nodes >/dev/null
"

compose exec -T control-plane-3 systemctl stop etcd.service

compose exec -T control-plane bash -lc "
set -Eeuo pipefail
timeout 30 bash -c 'until curl -sf http://localhost:8080/healthz >/dev/null; do sleep 1; done'
curl -sf http://localhost:8080/api/v1/nodes >/dev/null
ETCDCTL_API=3 etcdctl \
    --endpoints=https://control-plane:2379,https://control-plane-2:2379 \
    --cacert=/etc/tugboat/pki/etcd/ca.crt \
    --cert=/etc/tugboat/pki/etcd/client.crt \
    --key=/etc/tugboat/pki/etcd/client.key \
    endpoint health >/dev/null
"
