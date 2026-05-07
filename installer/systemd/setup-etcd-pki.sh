#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

PKI_DIR="/etc/tugboat/pki/etcd"
FORCE=0
SERVER_HOSTS=()
SERVER_IPS=()
PEER_HOSTS=()
PEER_IPS=()
WORK_DIR=""

usage() {
    cat <<'USAGE'
Usage: setup-etcd-pki.sh [options]

Options:
  --pki-dir <path>       etcd PKI output directory. Default: /etc/tugboat/pki/etcd.
  --server-host <name>   DNS SAN for the etcd server certificate. May be repeated.
  --server-ip <addr>     IP SAN for the etcd server certificate. May be repeated.
  --peer-host <name>     DNS SAN for the etcd peer certificate. May be repeated.
  --peer-ip <addr>       IP SAN for the etcd peer certificate. May be repeated.
  --force                Regenerate existing certificates and keys.
USAGE
}

parse_args() {
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --pki-dir)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--pki-dir requires a path."
                    return 2
                fi
                PKI_DIR="$2"
                shift 2
                ;;
            --pki-dir=*)
                PKI_DIR="${1#--pki-dir=}"
                if [[ -z "${PKI_DIR}" ]]; then
                    log_error "--pki-dir requires a path."
                    return 2
                fi
                shift
                ;;
            --server-host)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--server-host requires a name."
                    return 2
                fi
                SERVER_HOSTS+=("$2")
                shift 2
                ;;
            --server-host=*)
                if [[ -z "${1#--server-host=}" ]]; then
                    log_error "--server-host requires a name."
                    return 2
                fi
                SERVER_HOSTS+=("${1#--server-host=}")
                shift
                ;;
            --server-ip)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--server-ip requires an address."
                    return 2
                fi
                SERVER_IPS+=("$2")
                shift 2
                ;;
            --server-ip=*)
                if [[ -z "${1#--server-ip=}" ]]; then
                    log_error "--server-ip requires an address."
                    return 2
                fi
                SERVER_IPS+=("${1#--server-ip=}")
                shift
                ;;
            --peer-host)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--peer-host requires a name."
                    return 2
                fi
                PEER_HOSTS+=("$2")
                shift 2
                ;;
            --peer-host=*)
                if [[ -z "${1#--peer-host=}" ]]; then
                    log_error "--peer-host requires a name."
                    return 2
                fi
                PEER_HOSTS+=("${1#--peer-host=}")
                shift
                ;;
            --peer-ip)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--peer-ip requires an address."
                    return 2
                fi
                PEER_IPS+=("$2")
                shift 2
                ;;
            --peer-ip=*)
                if [[ -z "${1#--peer-ip=}" ]]; then
                    log_error "--peer-ip requires an address."
                    return 2
                fi
                PEER_IPS+=("${1#--peer-ip=}")
                shift
                ;;
            --force)
                FORCE=1
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

require_openssl() {
    if ! command -v openssl >/dev/null 2>&1; then
        log_error "openssl is required to generate etcd PKI."
        return 1
    fi
}

append_unique() {
    local -n values_ref="$1"
    local candidate="$2"
    local value

    if [[ -z "${candidate}" ]]; then
        return 0
    fi

    for value in "${values_ref[@]}"; do
        if [[ "${value}" == "${candidate}" ]]; then
            return 0
        fi
    done
    values_ref+=("${candidate}")
}

set_default_sans() {
    local fqdn_host
    local short_host

    append_unique SERVER_HOSTS localhost
    append_unique SERVER_IPS 127.0.0.1
    append_unique PEER_HOSTS localhost
    append_unique PEER_IPS 127.0.0.1

    short_host="$(hostname -s 2>/dev/null || true)"
    fqdn_host="$(hostname -f 2>/dev/null || true)"
    append_unique SERVER_HOSTS "${short_host}"
    append_unique SERVER_HOSTS "${fqdn_host}"
    append_unique PEER_HOSTS "${short_host}"
    append_unique PEER_HOSTS "${fqdn_host}"
}

all_outputs_exist() {
    [[ -f "${PKI_DIR}/ca.crt" &&
        -f "${PKI_DIR}/ca.key" &&
        -f "${PKI_DIR}/server.crt" &&
        -f "${PKI_DIR}/server.key" &&
        -f "${PKI_DIR}/peer.crt" &&
        -f "${PKI_DIR}/peer.key" &&
        -f "${PKI_DIR}/client.crt" &&
        -f "${PKI_DIR}/client.key" ]]
}

any_output_exists() {
    [[ -e "${PKI_DIR}/ca.crt" ||
        -e "${PKI_DIR}/ca.key" ||
        -e "${PKI_DIR}/server.crt" ||
        -e "${PKI_DIR}/server.key" ||
        -e "${PKI_DIR}/peer.crt" ||
        -e "${PKI_DIR}/peer.key" ||
        -e "${PKI_DIR}/client.crt" ||
        -e "${PKI_DIR}/client.key" ]]
}

set_pki_permissions() {
    chmod 0755 -- "${PKI_DIR}"
    chmod 0644 -- "${PKI_DIR}/ca.crt" "${PKI_DIR}/server.crt" "${PKI_DIR}/peer.crt" "${PKI_DIR}/client.crt"
    chmod 0600 -- "${PKI_DIR}/ca.key" "${PKI_DIR}/server.key" "${PKI_DIR}/peer.key" "${PKI_DIR}/client.key"
}

write_extfile() {
    local path="$1"
    local usage="$2"
    local hosts_name="$3"
    local ips_name="$4"
    local index=1
    local value
    local -n hosts_ref="${hosts_name}"
    local -n ips_ref="${ips_name}"

    {
        printf 'basicConstraints = CA:FALSE\n'
        printf 'keyUsage = digitalSignature, keyEncipherment\n'
        printf 'extendedKeyUsage = %s\n' "${usage}"
        if [[ "${#hosts_ref[@]}" -gt 0 || "${#ips_ref[@]}" -gt 0 ]]; then
            printf 'subjectAltName = @alt_names\n'
            printf '\n'
            printf '[alt_names]\n'
            for value in "${hosts_ref[@]}"; do
                printf 'DNS.%d = %s\n' "${index}" "${value}"
                index=$((index + 1))
            done
            index=1
            for value in "${ips_ref[@]}"; do
                printf 'IP.%d = %s\n' "${index}" "${value}"
                index=$((index + 1))
            done
        fi
    } > "${path}"
}

generate_leaf() {
    local name="$1"
    local common_name="$2"
    local extfile="$3"

    openssl genrsa -out "${PKI_DIR}/${name}.key" 4096
    openssl req -new \
        -key "${PKI_DIR}/${name}.key" \
        -subj "/CN=${common_name}" \
        -out "${WORK_DIR}/${name}.csr"
    openssl x509 -req \
        -in "${WORK_DIR}/${name}.csr" \
        -CA "${PKI_DIR}/ca.crt" \
        -CAkey "${PKI_DIR}/ca.key" \
        -CAcreateserial \
        -out "${PKI_DIR}/${name}.crt" \
        -days 3650 \
        -sha256 \
        -extfile "${extfile}"
}

generate_pki() {
    local client_ext
    local peer_ext
    local server_ext

    if [[ "${FORCE}" -ne 1 ]]; then
        if all_outputs_exist; then
            set_pki_permissions
            log_info "Existing etcd PKI found at ${PKI_DIR}; leaving it unchanged."
            return 0
        fi
        if any_output_exists; then
            log_error "Partial etcd PKI already exists in ${PKI_DIR}; use --force to regenerate."
            return 1
        fi
    fi

    mkdir -p -- "${PKI_DIR}"
    WORK_DIR="$(mktemp -d -t tugboat-etcd-pki.XXXXXXXXXX)"
    (
        trap 'rm -rf -- "${WORK_DIR}"' EXIT
        server_ext="${WORK_DIR}/server.ext"
        peer_ext="${WORK_DIR}/peer.ext"
        client_ext="${WORK_DIR}/client.ext"
        write_extfile "${server_ext}" serverAuth SERVER_HOSTS SERVER_IPS
        write_extfile "${peer_ext}" "serverAuth, clientAuth" PEER_HOSTS PEER_IPS
        write_extfile "${client_ext}" clientAuth SERVER_HOSTS SERVER_IPS

        openssl genrsa -out "${PKI_DIR}/ca.key" 4096
        openssl req -x509 -new -nodes \
            -key "${PKI_DIR}/ca.key" \
            -sha256 \
            -days 3650 \
            -subj "/CN=tugboat-etcd-ca" \
            -out "${PKI_DIR}/ca.crt"

        generate_leaf server tugboat-etcd-server "${server_ext}"
        generate_leaf peer tugboat-etcd-peer "${peer_ext}"
        generate_leaf client tugboat-etcd-client "${client_ext}"
        rm -f -- "${PKI_DIR}/ca.srl"
    )

    set_pki_permissions
    log_info "Generated etcd PKI in ${PKI_DIR}."
}

main() {
    parse_args "$@"
    require_openssl
    set_default_sans
    generate_pki
}

main "$@"
