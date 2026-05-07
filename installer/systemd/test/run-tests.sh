#!/usr/bin/env bash
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
INSTALLER_DIR="$(cd -- "${SCRIPT_DIR}/.." && pwd -P)"
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.test.yml"
PROJECT_NAME="${TUGBOAT_TEST_PROJECT_NAME:-tugboat-systemd-test}"
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

Runs installer/systemd tests. Scenario names may be passed with or without the
.sh suffix, for example: --scenario 01-control-plane-only.

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
    bash -n "${INSTALLER_DIR}/lib.sh"
    bash -n "${SCRIPT_DIR}/run-tests.sh"
    find "${SCRIPT_DIR}/scenarios" -maxdepth 1 -type f -name '*.sh' -print0 |
        xargs -0 -r bash -n

    if command -v shellcheck >/dev/null 2>&1; then
        shellcheck -x "${INSTALLER_DIR}/lib.sh" "${SCRIPT_DIR}/run-tests.sh" "${SCRIPT_DIR}"/scenarios/*.sh
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

scenario_requires_docker() {
    local scenario="$1"

    [[ "${scenario}" != "00-lib-unit.sh" ]]
}

missing_dependency() {
    local scenario="$1"

    case "${scenario}" in
        01-control-plane-only.sh)
            [[ -f "${INSTALLER_DIR}/install-control-plane.sh" ]] || printf '%s\n' "installer/systemd/install-control-plane.sh is not implemented yet"
            ;;
        02-worker-join.sh)
            [[ -f "${INSTALLER_DIR}/install-control-plane.sh" ]] || printf '%s\n' "installer/systemd/install-control-plane.sh is not implemented yet"
            [[ -f "${INSTALLER_DIR}/install-worker.sh" ]] || printf '%s\n' "installer/systemd/install-worker.sh is not implemented yet"
            ;;
        03-flannel-bootstrap.sh)
            [[ -f "${INSTALLER_DIR}/install-control-plane.sh" ]] || printf '%s\n' "installer/systemd/install-control-plane.sh is not implemented yet"
            [[ -f "${INSTALLER_DIR}/bootstrap-flannel.sh" ]] || printf '%s\n' "installer/systemd/bootstrap-flannel.sh is not implemented yet"
            ;;
        04-uninstall-purge.sh)
            [[ -f "${INSTALLER_DIR}/install-control-plane.sh" ]] || printf '%s\n' "installer/systemd/install-control-plane.sh is not implemented yet"
            [[ -f "${INSTALLER_DIR}/uninstall.sh" ]] || printf '%s\n' "installer/systemd/uninstall.sh is not implemented yet"
            ;;
        05-tls-pki.sh)
            [[ -f "${INSTALLER_DIR}/install-control-plane.sh" ]] || printf '%s\n' "installer/systemd/install-control-plane.sh is not implemented yet"
            [[ -f "${INSTALLER_DIR}/install-worker.sh" ]] || printf '%s\n' "installer/systemd/install-worker.sh is not implemented yet"
            [[ -f "${INSTALLER_DIR}/setup-pki.sh" ]] || printf '%s\n' "installer/systemd/setup-pki.sh is not implemented yet"
            ;;
    esac
}

require_docker() {
    if ! command -v docker >/dev/null 2>&1; then
        echo "docker is required for installer/systemd E2E scenarios" >&2
        exit 1
    fi

    docker compose version >/dev/null
}

build_release_binaries() {
    if [[ "${USE_PREBUILT}" -ne 1 ]]; then
        return 0
    fi

    cargo build --release "${BINARIES[@]/#/--package=}"
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

    compose exec -T control-plane bash -lc 'shopt -s nullglob; shellcheck -x /workspace/installer/systemd/*.sh /workspace/installer/systemd/test/run-tests.sh /workspace/installer/systemd/test/scenarios/*.sh'
}

copy_prebuilt_binaries() {
    local install_lines=""
    local binary

    for binary in "${BINARIES[@]}"; do
        install_lines+="install -m 0755 /workspace/target/release/${binary} ${PREBUILT_BIN_DIR}/${binary}; "
    done

    compose exec -T control-plane bash -lc "set -euo pipefail; mkdir -p ${PREBUILT_BIN_DIR}; ${install_lines}"
}

collect_journals() {
    local service

    if [[ "${ENV_STARTED}" -ne 1 ]]; then
        return 0
    fi

    mkdir -p -- "${ARTIFACT_DIR}"

    for service in control-plane worker; do
        if compose ps -q "${service}" >/dev/null 2>&1; then
            compose exec -T "${service}" bash -lc 'journalctl --no-pager -u "tugboat*" -u etcd.service || true; systemctl --failed --no-pager || true' \
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
    local reason

    if [[ ! -x "${path}" ]]; then
        echo "Scenario not found or not executable: ${scenario}" >&2
        exit 1
    fi

    printf '==> %s\n' "${scenario}"
    reason="$(missing_dependency "${scenario}")"
    if [[ -n "${reason}" ]]; then
        while IFS= read -r line; do
            printf 'skip: %s\n' "${line}"
        done <<< "${reason}"
        printf 'ok: %s\n' "${scenario}"
        return 0
    fi

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

for scenario in "${SCENARIOS[@]}"; do
    if scenario_requires_docker "${scenario}" && [[ -z "$(missing_dependency "${scenario}")" ]]; then
        trap cleanup EXIT
        start_environment
        break
    fi
done

export TUGBOAT_TEST_COMPOSE_FILE="${COMPOSE_FILE}"
export TUGBOAT_TEST_PROJECT_NAME="${PROJECT_NAME}"
export TUGBOAT_TEST_PREBUILT_BIN_DIR="${PREBUILT_BIN_DIR}"

for scenario in "${SCENARIOS[@]}"; do
    run_scenario "${scenario}"
done
