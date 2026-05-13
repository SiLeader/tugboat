#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

PKI_DIR="/etc/tugboat/pki"
FORCE=0
APISERVER_HOSTS=()
APISERVER_IPS=()
SA_SIGNING_ALGORITHM="rsa"

usage() {
    cat <<'USAGE'
Usage: setup-pki.sh [options]

Options:
  --pki-dir <path>             PKI output directory. Default: /etc/tugboat/pki.
  --apiserver-host <name>      DNS SAN for the apiserver certificate. May be repeated.
  --apiserver-ip <addr>        IP SAN for the apiserver certificate. May be repeated.
  --algorithm <rsa|ed25519>    ServiceAccount JWT signing key algorithm. Default: rsa.
                               Apiserver config must use signing_algorithm = "RS256" (rsa)
                               or "EdDSA" (ed25519) to match this choice.
  --force                      Regenerate existing certificates and keys.
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
            --apiserver-host)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--apiserver-host requires a name."
                    return 2
                fi
                APISERVER_HOSTS+=("$2")
                shift 2
                ;;
            --apiserver-host=*)
                if [[ -z "${1#--apiserver-host=}" ]]; then
                    log_error "--apiserver-host requires a name."
                    return 2
                fi
                APISERVER_HOSTS+=("${1#--apiserver-host=}")
                shift
                ;;
            --apiserver-ip)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--apiserver-ip requires an address."
                    return 2
                fi
                APISERVER_IPS+=("$2")
                shift 2
                ;;
            --apiserver-ip=*)
                if [[ -z "${1#--apiserver-ip=}" ]]; then
                    log_error "--apiserver-ip requires an address."
                    return 2
                fi
                APISERVER_IPS+=("${1#--apiserver-ip=}")
                shift
                ;;
            --algorithm)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--algorithm requires rsa or ed25519."
                    return 2
                fi
                SA_SIGNING_ALGORITHM="$2"
                shift 2
                ;;
            --algorithm=*)
                SA_SIGNING_ALGORITHM="${1#--algorithm=}"
                if [[ -z "${SA_SIGNING_ALGORITHM}" ]]; then
                    log_error "--algorithm requires rsa or ed25519."
                    return 2
                fi
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
        log_error "openssl is required to generate Tugboat PKI."
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
    local short_host
    local fqdn_host

    append_unique APISERVER_HOSTS localhost
    append_unique APISERVER_IPS 127.0.0.1

    short_host="$(hostname -s 2>/dev/null || true)"
    fqdn_host="$(hostname -f 2>/dev/null || true)"
    append_unique APISERVER_HOSTS "${short_host}"
    append_unique APISERVER_HOSTS "${fqdn_host}"
}

all_outputs_exist() {
    core_outputs_exist && service_account_signing_outputs_exist
}

core_outputs_exist() {
    [[ -f "${PKI_DIR}/ca.crt" &&
        -f "${PKI_DIR}/ca.key" &&
        -f "${PKI_DIR}/apiserver.crt" &&
        -f "${PKI_DIR}/apiserver.key" ]]
}

service_account_signing_outputs_exist() {
    [[ -f "${PKI_DIR}/tugboat-apiserver-sa-signing.key" &&
        -f "${PKI_DIR}/tugboat-apiserver-sa-signing.pub" ]]
}

any_output_exists() {
    [[ -e "${PKI_DIR}/ca.crt" ||
        -e "${PKI_DIR}/ca.key" ||
        -e "${PKI_DIR}/apiserver.crt" ||
        -e "${PKI_DIR}/apiserver.key" ||
        -e "${PKI_DIR}/tugboat-apiserver-sa-signing.key" ||
        -e "${PKI_DIR}/tugboat-apiserver-sa-signing.pub" ]]
}

any_service_account_signing_output_exists() {
    [[ -e "${PKI_DIR}/tugboat-apiserver-sa-signing.key" ||
        -e "${PKI_DIR}/tugboat-apiserver-sa-signing.pub" ]]
}

set_pki_permissions() {
    chmod 0755 -- "${PKI_DIR}"
    chmod 0644 -- "${PKI_DIR}/ca.crt" "${PKI_DIR}/apiserver.crt" "${PKI_DIR}/tugboat-apiserver-sa-signing.pub"
    chmod 0600 -- "${PKI_DIR}/ca.key" "${PKI_DIR}/apiserver.key" "${PKI_DIR}/tugboat-apiserver-sa-signing.key"
}

normalize_sa_signing_algorithm() {
    # Normalize to a lowercase token; the configuration file uses the JWT name
    # (RS256 / EdDSA), but operators sometimes pass those here too.
    case "${SA_SIGNING_ALGORITHM,,}" in
        rsa|rs256)
            SA_SIGNING_ALGORITHM="rsa"
            ;;
        ed25519|eddsa)
            SA_SIGNING_ALGORITHM="ed25519"
            ;;
        *)
            log_error "--algorithm must be one of: rsa, ed25519 (got: ${SA_SIGNING_ALGORITHM})."
            return 2
            ;;
    esac
}

generate_service_account_signing_key() {
    case "${SA_SIGNING_ALGORITHM}" in
        rsa)
            openssl genpkey -algorithm RSA \
                -pkeyopt rsa_keygen_bits:2048 \
                -out "${PKI_DIR}/tugboat-apiserver-sa-signing.key"
            ;;
        ed25519)
            openssl genpkey -algorithm ED25519 \
                -out "${PKI_DIR}/tugboat-apiserver-sa-signing.key"
            ;;
        *)
            log_error "Internal error: unsupported SA_SIGNING_ALGORITHM=${SA_SIGNING_ALGORITHM}."
            return 1
            ;;
    esac
    openssl pkey \
        -in "${PKI_DIR}/tugboat-apiserver-sa-signing.key" \
        -pubout \
        -out "${PKI_DIR}/tugboat-apiserver-sa-signing.pub"
}

write_extfile() {
    local path="$1"
    local index=1
    local value

    {
        printf 'basicConstraints = CA:FALSE\n'
        printf 'keyUsage = digitalSignature\n'
        printf 'extendedKeyUsage = serverAuth\n'
        printf 'subjectAltName = @alt_names\n'
        printf '\n'
        printf '[alt_names]\n'
        for value in "${APISERVER_HOSTS[@]}"; do
            printf 'DNS.%d = %s\n' "${index}" "${value}"
            index=$((index + 1))
        done
        index=1
        for value in "${APISERVER_IPS[@]}"; do
            printf 'IP.%d = %s\n' "${index}" "${value}"
            index=$((index + 1))
        done
    } > "${path}"
}

generate_pki() {
    local work_dir
    local extfile

    if [[ "${FORCE}" -ne 1 ]]; then
        if all_outputs_exist; then
            set_pki_permissions
            log_info "Existing Tugboat PKI found at ${PKI_DIR}; leaving it unchanged."
            return 0
        fi
        if core_outputs_exist; then
            if any_service_account_signing_output_exists; then
                log_error "Partial ServiceAccount signing key exists in ${PKI_DIR}; use --force to regenerate."
                return 1
            fi
            mkdir -p -- "${PKI_DIR}"
            (
                umask 077
                generate_service_account_signing_key
            )
            set_pki_permissions
            log_info "Generated Tugboat ServiceAccount signing key in ${PKI_DIR}."
            return 0
        fi
        if any_output_exists; then
            log_error "Partial PKI already exists in ${PKI_DIR}; use --force to regenerate."
            return 1
        fi
    fi

    mkdir -p -- "${PKI_DIR}"
    work_dir="$(mktemp -d -t tugboat-pki.XXXXXXXXXX)"
    (
        umask 077
        trap 'rm -rf -- "${work_dir}"' EXIT
        extfile="${work_dir}/apiserver.ext"
        write_extfile "${extfile}"

        openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "${PKI_DIR}/ca.key"
        openssl req -x509 -new -nodes \
            -key "${PKI_DIR}/ca.key" \
            -sha256 \
            -days 3650 \
            -subj "/CN=tugboat-ca" \
            -out "${PKI_DIR}/ca.crt"

        openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "${PKI_DIR}/apiserver.key"
        openssl req -new \
            -key "${PKI_DIR}/apiserver.key" \
            -subj "/CN=tugboat-apiserver" \
            -out "${work_dir}/apiserver.csr"
        openssl x509 -req \
            -in "${work_dir}/apiserver.csr" \
            -CA "${PKI_DIR}/ca.crt" \
            -CAkey "${PKI_DIR}/ca.key" \
            -CAcreateserial \
            -out "${PKI_DIR}/apiserver.crt" \
            -days 3650 \
            -sha256 \
            -extfile "${extfile}"
        rm -f -- "${PKI_DIR}/ca.srl"

        generate_service_account_signing_key
    )

    set_pki_permissions
    log_info "Generated Tugboat PKI in ${PKI_DIR}."
}

main() {
    parse_args "$@"
    normalize_sa_signing_algorithm
    require_openssl
    set_default_sans
    generate_pki
}

main "$@"
