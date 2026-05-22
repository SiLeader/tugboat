#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

control_plane_inventory false 0.0.0.0:8080 http://control-plane:8080 127.0.0.1:2379 https://127.0.0.1:2379 |
    run_playbook control-plane site.yml

compose exec -T control-plane bash -lc '
set -Eeuo pipefail
systemctl is-active --quiet etcd.service
systemctl is-active --quiet tugboat-apiserver.service
grep -q -- "--client-cert-auth" /etc/systemd/system/etcd.service
grep -q -- "--cert-file /etc/tugboat/pki/etcd/server.crt" /etc/systemd/system/etcd.service
grep -q "endpoints = \\[\"https://127.0.0.1:2379\"\\]" /etc/tugboat/apiserver/config.toml
grep -q "\\[etcd.tls\\]" /etc/tugboat/apiserver/config.toml
grep -q "cert_path = \"/etc/tugboat/pki/etcd/client.crt\"" /etc/tugboat/apiserver/config.toml

if ETCDCTL_API=3 etcdctl \
    --endpoints=https://127.0.0.1:2379 \
    --cacert=/etc/tugboat/pki/etcd/ca.crt \
    --dial-timeout=3s \
    endpoint health >/tmp/etcd-no-client-cert.out 2>&1; then
    echo "etcdctl connected without a client certificate" >&2
    exit 1
fi

ETCDCTL_API=3 etcdctl \
    --endpoints=https://127.0.0.1:2379 \
    --cacert=/etc/tugboat/pki/etcd/ca.crt \
    --cert=/etc/tugboat/pki/etcd/client.crt \
    --key=/etc/tugboat/pki/etcd/client.key \
    endpoint health >/dev/null

curl -sf http://localhost:8080/healthz >/dev/null
curl -sf http://localhost:8080/api/v1/nodes >/dev/null
'
