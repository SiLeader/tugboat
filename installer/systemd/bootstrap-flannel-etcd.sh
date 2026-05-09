#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

FLANNEL_SUBNET="10.244.0.0/16"
FLANNEL_BACKEND="vxlan"
FLANNEL_ETCD_ENDPOINTS="https://127.0.0.1:2379"
FLANNEL_ETCD_PREFIX="/coreos.com/network"
FLANNEL_ETCD_CA=""
FLANNEL_ETCD_CERT=""
FLANNEL_ETCD_KEY=""
CONFIG_KEY=""

usage() {
    cat <<'USAGE'
Usage: bootstrap-flannel-etcd.sh [options]

Options:
  --backend <vxlan|host-gw>       Flannel backend to write. Default: vxlan.
  --subnet <cidr>                 Flannel network CIDR. Default: 10.244.0.0/16.
  --flannel-etcd-endpoints <urls> Comma-separated etcd endpoints. Default: https://127.0.0.1:2379.
  --flannel-etcd-ca <path>        etcd TLS CA certificate.
  --flannel-etcd-cert <path>      etcd TLS client certificate.
  --flannel-etcd-key <path>       etcd TLS client private key.
USAGE
}

parse_args() {
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --backend)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--backend requires vxlan or host-gw."
                    return 2
                fi
                FLANNEL_BACKEND="$2"
                shift 2
                ;;
            --backend=*)
                FLANNEL_BACKEND="${1#--backend=}"
                shift
                ;;
            --subnet)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--subnet requires a CIDR."
                    return 2
                fi
                FLANNEL_SUBNET="$2"
                shift 2
                ;;
            --subnet=*)
                FLANNEL_SUBNET="${1#--subnet=}"
                shift
                ;;
            --flannel-etcd-endpoints)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--flannel-etcd-endpoints requires a URL list."
                    return 2
                fi
                FLANNEL_ETCD_ENDPOINTS="$2"
                shift 2
                ;;
            --flannel-etcd-endpoints=*)
                FLANNEL_ETCD_ENDPOINTS="${1#--flannel-etcd-endpoints=}"
                shift
                ;;
            --flannel-etcd-ca)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--flannel-etcd-ca requires a path."
                    return 2
                fi
                FLANNEL_ETCD_CA="$2"
                shift 2
                ;;
            --flannel-etcd-ca=*)
                FLANNEL_ETCD_CA="${1#--flannel-etcd-ca=}"
                shift
                ;;
            --flannel-etcd-cert)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--flannel-etcd-cert requires a path."
                    return 2
                fi
                FLANNEL_ETCD_CERT="$2"
                shift 2
                ;;
            --flannel-etcd-cert=*)
                FLANNEL_ETCD_CERT="${1#--flannel-etcd-cert=}"
                shift
                ;;
            --flannel-etcd-key)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--flannel-etcd-key requires a path."
                    return 2
                fi
                FLANNEL_ETCD_KEY="$2"
                shift 2
                ;;
            --flannel-etcd-key=*)
                FLANNEL_ETCD_KEY="${1#--flannel-etcd-key=}"
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
}

validate_args() {
    local path

    case "${FLANNEL_BACKEND}" in
        vxlan|host-gw)
            ;;
        *)
            log_error "--backend must be vxlan or host-gw."
            return 2
            ;;
    esac
    if [[ -z "${FLANNEL_SUBNET}" ]]; then
        log_error "--subnet must not be empty."
        return 2
    fi
    if [[ -z "${FLANNEL_ETCD_ENDPOINTS}" ]]; then
        log_error "--flannel-etcd-endpoints must not be empty."
        return 2
    fi
    if [[ -n "${FLANNEL_ETCD_CA}${FLANNEL_ETCD_CERT}${FLANNEL_ETCD_KEY}" ]]; then
        if [[ -z "${FLANNEL_ETCD_CA}" || -z "${FLANNEL_ETCD_CERT}" || -z "${FLANNEL_ETCD_KEY}" ]]; then
            log_error "--flannel-etcd-ca, --flannel-etcd-cert, and --flannel-etcd-key must be provided together."
            return 2
        fi
        for path in "${FLANNEL_ETCD_CA}" "${FLANNEL_ETCD_CERT}" "${FLANNEL_ETCD_KEY}"; do
            if [[ ! -f "${path}" ]]; then
                log_error "Flannel etcd TLS file not found: ${path}"
                return 2
            fi
        done
    fi

    CONFIG_KEY="${FLANNEL_ETCD_PREFIX%/}/config"
}

set_default_tls_if_present() {
    if [[ -n "${FLANNEL_ETCD_CA}${FLANNEL_ETCD_CERT}${FLANNEL_ETCD_KEY}" ]]; then
        return 0
    fi
    if [[ -f /etc/tugboat/pki/etcd/ca.crt &&
        -f /etc/tugboat/pki/etcd/client.crt &&
        -f /etc/tugboat/pki/etcd/client.key ]]; then
        FLANNEL_ETCD_CA="/etc/tugboat/pki/etcd/ca.crt"
        FLANNEL_ETCD_CERT="/etc/tugboat/pki/etcd/client.crt"
        FLANNEL_ETCD_KEY="/etc/tugboat/pki/etcd/client.key"
    fi
}

etcdctl_args() {
    printf '%s\0' --endpoints="${FLANNEL_ETCD_ENDPOINTS}"
    if [[ -n "${FLANNEL_ETCD_CA}" ]]; then
        printf '%s\0' \
            --cacert="${FLANNEL_ETCD_CA}" \
            --cert="${FLANNEL_ETCD_CERT}" \
            --key="${FLANNEL_ETCD_KEY}"
    fi
}

expected_config() {
    printf '{"Network":"%s","Backend":{"Type":"%s"}}' "${FLANNEL_SUBNET}" "${FLANNEL_BACKEND}"
}

configs_match() {
    local existing="$1"
    local expected="$2"

    if [[ "${existing}" == "${expected}" ]]; then
        return 0
    fi
    if ! command -v python3 >/dev/null 2>&1; then
        return 1
    fi

    python3 - "${existing}" "${expected}" <<'PY'
import json
import sys

try:
    existing = json.loads(sys.argv[1])
    expected = json.loads(sys.argv[2])
except json.JSONDecodeError:
    raise SystemExit(1)

if existing.get("Network") != expected.get("Network"):
    raise SystemExit(1)

existing_backend = existing.get("Backend") or {}
expected_backend = expected.get("Backend") or {}
if existing_backend.get("Type") != expected_backend.get("Type"):
    raise SystemExit(1)
PY
}

write_config() {
    local args=()
    local existing
    local expected

    while IFS= read -r -d '' arg; do
        args+=("${arg}")
    done < <(etcdctl_args)

    expected="$(expected_config)"
    existing="$(ETCDCTL_API=3 etcdctl "${args[@]}" get "${CONFIG_KEY}" --print-value-only)"
    if [[ -n "${existing}" ]]; then
        if configs_match "${existing}" "${expected}"; then
            log_info "Flannel etcd config already exists at ${CONFIG_KEY} and is compatible."
            return 0
        fi
        log_error "Existing Flannel etcd config at ${CONFIG_KEY} is incompatible: ${existing}"
        return 1
    fi

    ETCDCTL_API=3 etcdctl "${args[@]}" put "${CONFIG_KEY}" "${expected}" >/dev/null
    log_info "Wrote Flannel etcd config at ${CONFIG_KEY}."
}

main() {
    parse_args "$@"
    set_default_tls_if_present
    validate_args
    if ! command -v etcdctl >/dev/null 2>&1; then
        log_error "etcdctl is required to bootstrap Flannel network config."
        return 1
    fi
    write_config
}

main "$@"
