#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
# shellcheck source=installer/ansible/test/scenarios/lib.sh
source "${SCRIPT_DIR}/lib.sh"

reset_tugboat_cluster

inventory="$(
    control_plane_inventory false 0.0.0.0:8080 http://control-plane:8080 127.0.0.1:2379 https://127.0.0.1:2379
)"

run_playbook control-plane site.yml <<< "${inventory}"
second_output="$(run_playbook control-plane site.yml <<< "${inventory}")"
printf '%s\n' "${second_output}"
assert_no_ansible_changes "${second_output}"
