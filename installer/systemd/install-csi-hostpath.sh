#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

HOSTPATH_VERSION="v1.17.0"
HOSTPATH_BINARY=""
HOSTPATH_NODE_ID="$(hostname -s)"
HOSTPATH_DATA_DIR="/var/lib/tugboat-csi-hostpath"
INSTALL_MODE="build"

usage() {
    cat <<'USAGE'
Usage: install-csi-hostpath.sh [--build | --binary <path>] [options]

Options:
  --build             Build hostpathplugin from the release source. This is the default.
  --binary <path>     Install an existing hostpathplugin binary instead of building.
  --version <version> Kubernetes CSI hostpath version. Default: v1.17.0.
  --node-id <name>    CSI node id passed to hostpathplugin. Default: hostname -s.
  --data-dir <path>   Hostpath state and volume data directory. Default: /var/lib/tugboat-csi-hostpath.
USAGE
}

parse_args() {
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --build)
                INSTALL_MODE="build"
                shift
                ;;
            --binary)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--binary requires a path."
                    return 2
                fi
                INSTALL_MODE="binary"
                HOSTPATH_BINARY="$2"
                shift 2
                ;;
            --binary=*)
                INSTALL_MODE="binary"
                HOSTPATH_BINARY="${1#--binary=}"
                if [[ -z "${HOSTPATH_BINARY}" ]]; then
                    log_error "--binary requires a path."
                    return 2
                fi
                shift
                ;;
            --version)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--version requires a value."
                    return 2
                fi
                HOSTPATH_VERSION="$2"
                shift 2
                ;;
            --version=*)
                HOSTPATH_VERSION="${1#--version=}"
                if [[ -z "${HOSTPATH_VERSION}" ]]; then
                    log_error "--version requires a value."
                    return 2
                fi
                shift
                ;;
            --node-id)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--node-id requires a value."
                    return 2
                fi
                HOSTPATH_NODE_ID="$2"
                shift 2
                ;;
            --node-id=*)
                HOSTPATH_NODE_ID="${1#--node-id=}"
                if [[ -z "${HOSTPATH_NODE_ID}" ]]; then
                    log_error "--node-id requires a value."
                    return 2
                fi
                shift
                ;;
            --data-dir)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--data-dir requires a path."
                    return 2
                fi
                HOSTPATH_DATA_DIR="$2"
                shift 2
                ;;
            --data-dir=*)
                HOSTPATH_DATA_DIR="${1#--data-dir=}"
                if [[ -z "${HOSTPATH_DATA_DIR}" ]]; then
                    log_error "--data-dir requires a path."
                    return 2
                fi
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

    if [[ "${INSTALL_MODE}" == "binary" && ! -f "${HOSTPATH_BINARY}" ]]; then
        log_error "hostpathplugin binary not found: ${HOSTPATH_BINARY}"
        return 1
    fi
}

install_go_toolchain() {
    if command -v go >/dev/null 2>&1; then
        return 0
    fi

    apt-get update
    apt-get install -y golang-go
}

build_hostpathplugin() {
    local work_dir
    local source_dir

    install_go_toolchain
    work_dir="$(mktemp -d -t tugboat-hostpathplugin.XXXXXXXXXX)"
    (
        trap 'rm -rf -- "${work_dir}"' EXIT
        curl -fsSL "https://github.com/kubernetes-csi/csi-driver-host-path/archive/refs/tags/${HOSTPATH_VERSION}.tar.gz" |
            tar xzf - -C "${work_dir}"
        source_dir="${work_dir}/csi-driver-host-path-${HOSTPATH_VERSION#v}"
        cd -- "${source_dir}" || exit 1
        go build -o "${work_dir}/hostpathplugin" ./cmd/hostpathplugin
        install -m 0755 -- "${work_dir}/hostpathplugin" /usr/local/bin/hostpathplugin
    )
    log_info "Installed hostpathplugin ${HOSTPATH_VERSION} to /usr/local/bin/hostpathplugin"
}

install_hostpathplugin_binary() {
    install -m 0755 -- "${HOSTPATH_BINARY}" /usr/local/bin/hostpathplugin
    log_info "Installed hostpathplugin from ${HOSTPATH_BINARY}"
}

install_directories() {
    install -d -m 0755 -o tugboat-csi-hostpath -g tugboat-csi-hostpath -- /var/run/csi
    install -d -m 0755 -o tugboat-csi-hostpath -g tugboat-csi-hostpath -- "${HOSTPATH_DATA_DIR}"
}

install_hostpath_unit() {
    export HOSTPATH_NODE_ID HOSTPATH_DATA_DIR
    install_unit "${SCRIPT_DIR}/units/hostpath-provisioner.service.tpl" hostpath-provisioner.service
}

main() {
    require_root
    parse_args "$@"

    apt-get update
    apt-get install -y ca-certificates gettext-base
    create_system_user tugboat-csi-hostpath

    case "${INSTALL_MODE}" in
        build)
            build_hostpathplugin
            ;;
        binary)
            install_hostpathplugin_binary
            ;;
    esac

    install_directories
    install_hostpath_unit
    enable_unit hostpath-provisioner.service
}

main "$@"
