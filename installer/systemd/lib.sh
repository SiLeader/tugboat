#!/usr/bin/env bash

INSTALLER_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
: "${CARGO_MANIFEST_DIR:=$(cd -- "${INSTALLER_DIR}/../.." && pwd -P)}"
: "${SYSTEMD_UNIT_DIR:=/etc/systemd/system}"

BUILD_MODE="build"
PREBUILT_BIN_DIR=""

_log() {
    local color="$1"
    local level="$2"
    local message="$3"

    printf '\033[%sm[%s]\033[0m %s\n' "$color" "$level" "$message" >&2
}

log_info() {
    _log "1;34" "INFO" "$*"
}

log_warn() {
    _log "1;33" "WARN" "$*"
}

log_error() {
    _log "1;31" "ERROR" "$*"
}

require_root() {
    if [[ "${EUID}" -ne 0 ]]; then
        log_error "This script must be run as root."
        exit 1
    fi
}

parse_build_mode() {
    local selected_mode=""

    BUILD_MODE=""
    PREBUILT_BIN_DIR=""

    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --build)
                if [[ "${selected_mode}" == "prebuilt" ]]; then
                    log_error "--build and --use-prebuilt are mutually exclusive."
                    return 2
                fi
                selected_mode="build"
                PREBUILT_BIN_DIR=""
                shift
                ;;
            --use-prebuilt)
                if [[ "${selected_mode}" == "build" ]]; then
                    log_error "--build and --use-prebuilt are mutually exclusive."
                    return 2
                fi
                selected_mode="prebuilt"
                shift
                ;;
            --bin-dir)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--bin-dir requires a path."
                    return 2
                fi
                PREBUILT_BIN_DIR="$2"
                shift 2
                ;;
            --bin-dir=*)
                PREBUILT_BIN_DIR="${1#--bin-dir=}"
                if [[ -z "${PREBUILT_BIN_DIR}" ]]; then
                    log_error "--bin-dir requires a path."
                    return 2
                fi
                shift
                ;;
            *)
                shift
                ;;
        esac
    done

    if [[ -z "${selected_mode}" ]]; then
        log_error "Either --build or --use-prebuilt is required."
        return 2
    fi

    BUILD_MODE="${selected_mode}"

    if [[ "${BUILD_MODE}" == "prebuilt" && -z "${PREBUILT_BIN_DIR}" ]]; then
        log_error "--use-prebuilt requires --bin-dir."
        return 2
    fi

    if [[ "${BUILD_MODE}" == "build" && -n "${PREBUILT_BIN_DIR}" ]]; then
        log_error "--bin-dir can only be used with --use-prebuilt."
        return 2
    fi
}

build_binary() {
    local package="${1:-}"

    if [[ -z "${package}" ]]; then
        log_error "build_binary requires a package name."
        return 2
    fi

    log_info "Building ${package}"
    (
        cd -- "${CARGO_MANIFEST_DIR}" || exit
        cargo build --release --package "${package}"
    )
}

install_binary() {
    local package="${1:-}"
    local dest_dir="${2:-}"
    local source_path
    local dest_path

    if [[ -z "${package}" || -z "${dest_dir}" ]]; then
        log_error "install_binary requires a package name and destination directory."
        return 2
    fi

    mkdir -p -- "${dest_dir}"

    case "${BUILD_MODE}" in
        build)
            build_binary "${package}"
            source_path="${CARGO_MANIFEST_DIR}/target/release/${package}"
            ;;
        prebuilt)
            if [[ -z "${PREBUILT_BIN_DIR}" ]]; then
                log_error "PREBUILT_BIN_DIR must be set in prebuilt mode."
                return 2
            fi
            source_path="${PREBUILT_BIN_DIR}/${package}"
            ;;
        *)
            log_error "Unknown BUILD_MODE: ${BUILD_MODE}"
            return 2
            ;;
    esac

    if [[ ! -f "${source_path}" ]]; then
        log_error "Binary not found: ${source_path}"
        return 1
    fi

    dest_path="${dest_dir}/${package}"
    install -m 0755 -- "${source_path}" "${dest_path}"
    log_info "Installed ${package} to ${dest_path}"
}

fetch_tarball() {
    local url="${1:-}"
    local sha256="${2:-}"
    local dest_dir="${3:-}"
    local work_dir

    if [[ -z "${url}" || -z "${sha256}" || -z "${dest_dir}" ]]; then
        log_error "fetch_tarball requires a URL, sha256, and destination directory."
        return 2
    fi

    work_dir="$(mktemp -d -t tugboat-fetch.XXXXXXXXXX)"

    (
        trap 'rm -rf -- "${work_dir}"' EXIT
        mkdir -p -- "${dest_dir}"
        cd -- "${work_dir}" || exit
        wget -O archive.tar.gz "${url}"
        printf '%s  %s\n' "${sha256}" "archive.tar.gz" | sha256sum --check --status
        tar xzf archive.tar.gz -C "${dest_dir}"
    )
}

require_envsubst() {
    if command -v envsubst >/dev/null 2>&1; then
        return 0
    fi

    if ! command -v apt-get >/dev/null 2>&1; then
        log_error "envsubst is required, but apt-get is not available to install gettext-base."
        return 1
    fi

    require_root
    log_warn "envsubst not found; installing gettext-base."
    apt-get update
    apt-get install -y gettext-base
}

render_template() {
    local src="${1:-}"
    local dst="${2:-}"

    if [[ -z "${src}" || -z "${dst}" ]]; then
        log_error "render_template requires source and destination paths."
        return 2
    fi

    require_envsubst
    mkdir -p -- "$(dirname -- "${dst}")"
    envsubst < "${src}" > "${dst}"
}

create_system_user() {
    local name="${1:-}"

    if [[ -z "${name}" ]]; then
        log_error "create_system_user requires a user name."
        return 2
    fi

    if id -u "${name}" >/dev/null 2>&1; then
        log_info "System user already exists: ${name}"
        return 0
    fi

    useradd --system --no-create-home --shell /usr/sbin/nologin "${name}"
    log_info "Created system user: ${name}"
}

install_unit() {
    local tpl="${1:-}"
    local dst="${2:-}"
    local target_path

    if [[ -z "${tpl}" || -z "${dst}" ]]; then
        log_error "install_unit requires a template path and destination name or path."
        return 2
    fi

    if [[ "${dst}" == /* ]]; then
        target_path="${dst}"
    else
        target_path="${SYSTEMD_UNIT_DIR}/${dst}"
    fi

    render_template "${tpl}" "${target_path}"
    chmod 0644 -- "${target_path}"
    log_info "Installed systemd unit: ${target_path}"
}

enable_unit() {
    local name="${1:-}"

    if [[ -z "${name}" ]]; then
        log_error "enable_unit requires a unit name."
        return 2
    fi

    systemctl daemon-reload
    systemctl enable --now "${name}"
}

disable_unit() {
    local name="${1:-}"

    if [[ -z "${name}" ]]; then
        log_error "disable_unit requires a unit name."
        return 2
    fi

    systemctl disable --now "${name}" 2>/dev/null || true
}
