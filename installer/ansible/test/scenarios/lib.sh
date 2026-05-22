#!/usr/bin/env bash

: "${TUGBOAT_ANSIBLE_TEST_COMPOSE_FILE:?}"
: "${TUGBOAT_ANSIBLE_TEST_PROJECT_NAME:?}"
: "${TUGBOAT_ANSIBLE_TEST_PREBUILT_BIN_DIR:?}"

compose() {
    docker compose -f "${TUGBOAT_ANSIBLE_TEST_COMPOSE_FILE}" -p "${TUGBOAT_ANSIBLE_TEST_PROJECT_NAME}" "$@"
}

run_playbook() {
    local service="$1"
    local playbook="$2"
    shift 2

    compose exec -T "${service}" bash -lc '
set -Eeuo pipefail
mkdir -p /tmp/.ansible/tmp
cat > /tmp/tugboat-ansible-inventory.yml
cd /workspace/installer/ansible
ANSIBLE_LOCAL_TEMP=/tmp/.ansible/tmp \
ANSIBLE_REMOTE_TEMP=/tmp/.ansible/tmp \
ANSIBLE_CALLBACK_PLUGINS=/workspace/installer/ansible/test/callback_plugins \
ANSIBLE_CALLBACKS_ENABLED=tugboat_stats \
ansible-playbook -i /tmp/tugboat-ansible-inventory.yml "$@"
' _ "${playbook}" "$@"
}

reset_tugboat_host() {
    local service="$1"

    compose exec -T "${service}" bash -lc '
set -Eeuo pipefail
cd /workspace
if [[ -f installer/systemd/uninstall.sh ]]; then
    bash installer/systemd/uninstall.sh --control-plane --worker --csi-hostpath --purge >/dev/null 2>&1 || true
fi
rm -rf /var/lib/tugboat/ansible /tmp/tugboat-*
systemctl reset-failed >/dev/null 2>&1 || true
'
}

reset_tugboat_cluster() {
    reset_tugboat_host control-plane
    reset_tugboat_host worker
}

control_plane_inventory() {
    local secure="$1"
    local listen="$2"
    local advertise_url="$3"
    local etcd_listen="$4"
    local etcd_advertise_url="$5"
    local extra_vars="${6:-}"

    cat <<YAML
---
all:
  children:
    tugboat_control_plane:
      hosts:
        localhost:
          ansible_connection: local
          ansible_become: false
          tugboat_build_mode: prebuilt
          tugboat_prebuilt_bin_dir: ${TUGBOAT_ANSIBLE_TEST_PREBUILT_BIN_DIR}
          tugboat_cargo_manifest_dir: /workspace
          tugboat_secure: ${secure}
          tugboat_apiserver_listen: ${listen}
          tugboat_apiserver_advertise_url: ${advertise_url}
          tugboat_etcd_listen: ${etcd_listen}
          tugboat_etcd_peer_listen: 127.0.0.1:2380
          tugboat_etcd_node_name: control-plane
          tugboat_etcd_advertise_client_url: ${etcd_advertise_url}
          tugboat_etcd_initial_advertise_peer_url: https://127.0.0.1:2380
          tugboat_etcd_initial_cluster: control-plane=https://127.0.0.1:2380
          tugboat_etcd_endpoints:
            - ${etcd_advertise_url}
          tugboat_apiserver_cert_hosts:
            - localhost
            - control-plane
          tugboat_apiserver_cert_ips:
            - 127.0.0.1
${extra_vars}
    tugboat_workers:
      hosts: {}
    tugboat_csi_hostpath:
      hosts: {}
YAML
}

worker_inventory() {
    local secure="$1"
    local apiserver_url="$2"
    local extra_vars="${3:-}"

    cat <<YAML
---
all:
  children:
    tugboat_control_plane:
      hosts: {}
    tugboat_workers:
      hosts:
        localhost:
          ansible_connection: local
          ansible_become: false
          tugboat_build_mode: prebuilt
          tugboat_prebuilt_bin_dir: ${TUGBOAT_ANSIBLE_TEST_PREBUILT_BIN_DIR}
          tugboat_cargo_manifest_dir: /workspace
          tugboat_secure: ${secure}
          tugboat_apiserver_advertise_url: ${apiserver_url}
          tugboat_node_name: worker
          tugboat_worker_runtime: qemu
${extra_vars}
    tugboat_csi_hostpath:
      hosts: {}
YAML
}

assert_no_ansible_changes() {
    local output="$1"

    python3 - <<'PY' "${output}"
import json
import re
import sys

match = re.search(r"^TUGBOAT_ANSIBLE_STATS_JSON=(\{.*\})$", sys.argv[1], re.M)
if not match:
    raise SystemExit("Ansible stats callback output was not found")

stats = json.loads(match.group(1))
changed = {host: summary.get("changed", 0) for host, summary in stats.items()}
unexpected = {host: count for host, count in changed.items() if count != 0}
if unexpected:
    raise SystemExit(f"second Ansible run reported changes: {unexpected}")
PY
}
