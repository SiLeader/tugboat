#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
ANSIBLE_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
REPO_DIR="$(cd -- "${ANSIBLE_DIR}/../.." && pwd -P)"
SYSTEMD_TEST_DIR="${REPO_DIR}/installer/systemd/test"
COMPOSE_FILE="${SYSTEMD_TEST_DIR}/docker-compose.test.yml"
PROJECT_NAME="${TUGBOAT_ANSIBLE_TEST_PROJECT_NAME:-tugboat-ansible-test}"
ARTIFACT_DIR="${SCRIPT_DIR}/artifacts"
PREBUILT_BIN_DIR="/var/cache/tugboat-prebuilt"
SCENARIO=""
KEEP=0
USE_PREBUILT=1
ENV_STARTED=0

BINARIES=(
    tugboat-apiserver
    tugboat-scheduler
    tugboat-controller-manager
    tugboat-agent
    tugboat-qemu-runtime
    tugboat-cloud-hypervisor-runtime
)

usage() {
    cat <<'USAGE'
Usage: run-tests.sh [--scenario <name>] [--keep] [--use-prebuilt]

Runs installer/ansible tests in the Docker systemd environment. Scenario names
may be passed with or without the .sh suffix, for example:
--scenario 01-control-plane-only.

Options:
  --scenario <name>  Run one scenario instead of every scenario.
  --keep             Keep Docker containers and volumes after the run.
  --use-prebuilt     Build host release binaries and expose them in containers.
USAGE
}

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --scenario)
            if [[ "$#" -lt 2 || -z "$2" ]]; then
                echo "--scenario requires a scenario name" >&2
                exit 2
            fi
            SCENARIO="$2"
            shift 2
            ;;
        --keep)
            KEEP=1
            shift
            ;;
        --use-prebuilt)
            USE_PREBUILT=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

compose() {
    docker compose -f "${COMPOSE_FILE}" -p "${PROJECT_NAME}" "$@"
}

run_lint() {
    bash -n "${SCRIPT_DIR}/run-tests.sh"
    find "${SCRIPT_DIR}/scenarios" -maxdepth 1 -type f -name '*.sh' -print0 |
        xargs -0 -r bash -n

    if ! command -v ansible-playbook >/dev/null 2>&1; then
        echo "ansible-playbook is required for Ansible installer syntax checks" >&2
        exit 1
    fi

    (
        cd "${ANSIBLE_DIR}"
        ansible-playbook --syntax-check -i test/inventory.syntax.yml site.yml
        ansible-playbook --syntax-check -i test/inventory.syntax.yml uninstall.yml
    )

    if command -v shellcheck >/dev/null 2>&1; then
        shellcheck -x "${SCRIPT_DIR}/run-tests.sh" "${SCRIPT_DIR}"/scenarios/*.sh
    else
        echo "shellcheck not found; skipping shellcheck lint" >&2
    fi
}

normalize_scenario() {
    local name="$1"

    if [[ "${name}" == *.sh ]]; then
        printf '%s\n' "${name}"
    else
        printf '%s.sh\n' "${name}"
    fi
}

all_scenarios() {
    find "${SCRIPT_DIR}/scenarios" -maxdepth 1 -type f -name '[0-9][0-9]-*.sh' -printf '%f\n' | sort
}

require_docker() {
    if ! command -v docker >/dev/null 2>&1; then
        echo "docker is required for installer/ansible E2E scenarios" >&2
        exit 1
    fi

    docker compose version >/dev/null
}

build_release_binaries() {
    local binary

    if [[ "${USE_PREBUILT}" -ne 1 ]]; then
        return 0
    fi

    for binary in "${BINARIES[@]}"; do
        if [[ ! -x "${REPO_DIR}/target/release/${binary}" ]]; then
            cargo build --release "${BINARIES[@]/#/--package=}"
            return 0
        fi
    done

    echo "Using existing release binaries from target/release"
}

copy_prebuilt_binaries() {
    local install_lines=""
    local binary

    for binary in "${BINARIES[@]}"; do
        install_lines+="install -m 0755 /workspace/target/release/${binary} ${PREBUILT_BIN_DIR}/${binary}; "
    done

    compose exec -T control-plane bash -lc "set -euo pipefail; mkdir -p ${PREBUILT_BIN_DIR}; ${install_lines}"
}

start_environment() {
    require_docker
    build_release_binaries

    compose build
    compose up -d control-plane worker
    ENV_STARTED=1

    compose exec -T control-plane bash -lc "timeout 60 bash -c 'until state=\$(systemctl is-system-running); [[ \"\${state}\" == running || \"\${state}\" == degraded ]]; do sleep 1; done'"
    compose exec -T worker bash -lc "timeout 60 bash -c 'until state=\$(systemctl is-system-running); [[ \"\${state}\" == running || \"\${state}\" == degraded ]]; do sleep 1; done'"

    if [[ "${USE_PREBUILT}" -eq 1 ]]; then
        copy_prebuilt_binaries
    fi
}

collect_journals() {
    local service

    if [[ "${ENV_STARTED}" -ne 1 ]]; then
        return 0
    fi

    mkdir -p -- "${ARTIFACT_DIR}"

    for service in control-plane worker; do
        if compose ps -q "${service}" >/dev/null 2>&1; then
            compose exec -T "${service}" bash -lc 'journalctl --no-pager -u "tugboat*" -u etcd.service -u flanneld.service -u hostpath-provisioner.service || true; systemctl --failed --no-pager || true' \
                > "${ARTIFACT_DIR}/${service}-journal.log" 2>&1 || true
        fi
    done
}

cleanup() {
    local exit_code="$?"

    collect_journals || true

    if [[ "${ENV_STARTED}" -eq 1 && "${KEEP}" -ne 1 ]]; then
        compose down -v >/dev/null 2>&1 || true
    fi

    exit "${exit_code}"
}

run_scenario() {
    local scenario="$1"
    local path="${SCRIPT_DIR}/scenarios/${scenario}"

    if [[ ! -x "${path}" ]]; then
        echo "Scenario not found or not executable: ${scenario}" >&2
        exit 1
    fi

    printf '==> %s\n' "${scenario}"
    "${path}"
    printf 'ok: %s\n' "${scenario}"
}

run_lint

SCENARIOS=()
if [[ -n "${SCENARIO}" ]]; then
    SCENARIOS=("$(normalize_scenario "${SCENARIO}")")
else
    while IFS= read -r scenario; do
        SCENARIOS+=("${scenario}")
    done < <(all_scenarios)
fi

trap cleanup EXIT
start_environment

export TUGBOAT_ANSIBLE_TEST_COMPOSE_FILE="${COMPOSE_FILE}"
export TUGBOAT_ANSIBLE_TEST_PROJECT_NAME="${PROJECT_NAME}"
export TUGBOAT_ANSIBLE_TEST_PREBUILT_BIN_DIR="${PREBUILT_BIN_DIR}"

for scenario in "${SCENARIOS[@]}"; do
    run_scenario "${scenario}"
done
