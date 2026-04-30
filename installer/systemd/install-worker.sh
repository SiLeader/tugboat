#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

APISERVER_URL=""
NODE_NAME="$(hostname -s)"
RUNTIME="qemu"
CNI_SUBNET="10.244.0.0/16"
RUNTIME_BINARY="tugboat-qemu-runtime"
RUNTIME_CONFIG_FILE="config.toml"

CLOUD_HYPERVISOR_URL="https://github.com/cloud-hypervisor/cloud-hypervisor/releases/latest/download/cloud-hypervisor-static"
CLOUD_HYPERVISOR_SHA256SUMS_URL="https://github.com/cloud-hypervisor/cloud-hypervisor/releases/latest/download/SHA256SUMS"

usage() {
    cat <<'USAGE'
Usage: install-worker.sh (--build | --use-prebuilt --bin-dir <path>) --apiserver-url <url> [options]

Options:
  --build                         Build Tugboat binaries with cargo.
  --use-prebuilt                  Use binaries from --bin-dir.
  --bin-dir <path>                Directory containing prebuilt Tugboat binaries.
  --apiserver-url <url>           API server URL, for example http://192.168.0.1:8080. Required.
  --node-name <name>              Tugboat node name. Default: hostname -s.
  --runtime <qemu|cloud-hypervisor>
                                  VM runtime. Default: qemu.
  --cni-subnet <cidr>             CIDR written to /run/flannel/subnet.env. Default: 10.244.0.0/16.
USAGE
}

parse_worker_args() {
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --build|--use-prebuilt)
                shift
                ;;
            --bin-dir)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--bin-dir requires a path."
                    return 2
                fi
                shift 2
                ;;
            --bin-dir=*)
                shift
                ;;
            --apiserver-url)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--apiserver-url requires a URL."
                    return 2
                fi
                APISERVER_URL="$2"
                shift 2
                ;;
            --apiserver-url=*)
                APISERVER_URL="${1#--apiserver-url=}"
                shift
                ;;
            --node-name)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--node-name requires a name."
                    return 2
                fi
                NODE_NAME="$2"
                shift 2
                ;;
            --node-name=*)
                NODE_NAME="${1#--node-name=}"
                shift
                ;;
            --runtime)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--runtime requires qemu or cloud-hypervisor."
                    return 2
                fi
                RUNTIME="$2"
                shift 2
                ;;
            --runtime=*)
                RUNTIME="${1#--runtime=}"
                shift
                ;;
            --cni-subnet)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--cni-subnet requires a CIDR."
                    return 2
                fi
                CNI_SUBNET="$2"
                shift 2
                ;;
            --cni-subnet=*)
                CNI_SUBNET="${1#--cni-subnet=}"
                shift
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                log_error "Unknown argument: $1"
                usage >&2
                return 2
                ;;
        esac
    done

    if [[ -z "${APISERVER_URL}" ]]; then
        log_error "--apiserver-url is required."
        usage >&2
        return 2
    fi

    case "${RUNTIME}" in
        qemu)
            RUNTIME_BINARY="tugboat-qemu-runtime"
            RUNTIME_CONFIG_FILE="config.toml"
            ;;
        cloud-hypervisor)
            RUNTIME_BINARY="tugboat-cloud-hypervisor-runtime"
            RUNTIME_CONFIG_FILE="cloud-hypervisor-config.toml"
            ;;
        *)
            log_error "--runtime must be qemu or cloud-hypervisor."
            return 2
            ;;
    esac
}

install_base_packages() {
    apt-get update
    apt-get install -y ca-certificates curl gettext-base tar wget
}

install_qemu_dependencies() {
    apt-get install -y qemu-system-x86 qemu-utils ovmf
}

install_cloud_hypervisor_from_package() {
    if ! apt-get install -y cloud-hypervisor; then
        return 1
    fi

    if ! command -v cloud-hypervisor >/dev/null 2>&1; then
        log_warn "cloud-hypervisor package installed, but binary was not found on PATH."
        return 1
    fi

    install -m 0755 -- "$(command -v cloud-hypervisor)" /usr/local/bin/cloud-hypervisor
}

install_cloud_hypervisor_from_upstream() {
    local work_dir
    local checksum

    work_dir="$(mktemp -d -t tugboat-cloud-hypervisor.XXXXXXXXXX)"
    (
        trap 'rm -rf -- "${work_dir}"' EXIT
        cd -- "${work_dir}" || exit 1
        curl -fsSL -o cloud-hypervisor "${CLOUD_HYPERVISOR_URL}"
        curl -fsSL -o SHA256SUMS "${CLOUD_HYPERVISOR_SHA256SUMS_URL}"
        checksum="$(awk '$2 == "cloud-hypervisor-static" || $2 == "./cloud-hypervisor-static" { print $1 }' SHA256SUMS)"
        if [[ -z "${checksum}" ]]; then
            log_error "Could not find cloud-hypervisor-static checksum in upstream SHA256SUMS."
            exit 1
        fi
        printf '%s  %s\n' "${checksum}" "cloud-hypervisor" | sha256sum --check --status
        install -m 0755 -- cloud-hypervisor /usr/local/bin/cloud-hypervisor
    )
}

install_cloud_hypervisor_dependencies() {
    if install_cloud_hypervisor_from_package; then
        log_info "Installed Cloud Hypervisor from OS package."
        return 0
    fi

    log_warn "Falling back to upstream Cloud Hypervisor static binary."
    install_cloud_hypervisor_from_upstream
}

install_runtime_dependencies() {
    case "${RUNTIME}" in
        qemu)
            install_qemu_dependencies
            ;;
        cloud-hypervisor)
            install_cloud_hypervisor_dependencies
            ;;
    esac
}

install_runtime_binary() {
    case "${RUNTIME}" in
        qemu)
            install_binary tugboat-qemu-runtime /usr/local/bin
            ;;
        cloud-hypervisor)
            install_binary tugboat-cloud-hypervisor-runtime /usr/local/bin
            ;;
    esac
}

render_configs() {
    install -d -m 0755 -- /etc/tugboat/agent /etc/tugboat/runtime
    install -d -m 0755 -- /var/lib/tugboat-agent/images /var/lib/tugboat-agent/csi

    export APISERVER_URL NODE_NAME RUNTIME_BINARY RUNTIME_CONFIG_FILE
    render_template \
        "${INSTALLER_DIR}/configs/agent.config.toml.tpl" \
        /etc/tugboat/agent/config.toml

    case "${RUNTIME}" in
        qemu)
            render_template \
                "${INSTALLER_DIR}/configs/runtime-qemu.config.toml.tpl" \
                /etc/tugboat/runtime/config.toml
            ;;
        cloud-hypervisor)
            render_template \
                "${INSTALLER_DIR}/configs/runtime-cloud-hypervisor.config.toml.tpl" \
                /etc/tugboat/runtime/cloud-hypervisor-config.toml
            ;;
    esac
}

install_units() {
    install_unit "${INSTALLER_DIR}/units/tugboat-agent.service.tpl" tugboat-agent.service
}

print_registration_hint() {
    local nodes_url="${APISERVER_URL%/}/api/v1/nodes"

    log_info "To verify node registration, run:"
    printf '  curl %s\n' "${nodes_url}"
}

main() {
    require_root
    parse_build_mode "$@"
    parse_worker_args "$@"

    install_base_packages
    install_runtime_dependencies
    create_system_user tugboat

    install_binary tugboat-agent /usr/local/bin
    install_runtime_binary

    install_cni_plugins "${CNI_SUBNET}"
    render_configs
    install_units

    enable_unit tugboat-agent.service
    print_registration_hint
}

main "$@"
