#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

APISERVER_LISTEN="0.0.0.0:8080"
ETCD_LISTEN="127.0.0.1:2379"
DATA_DIR="/var/lib/tugboat-etcd"
PKI_DIR="/etc/tugboat/pki"
SECURE=0
FORCE_PKI=0
LISTEN_SET=0
APISERVER_CERT_HOSTS=()
APISERVER_CERT_IPS=()

ETCD_VERSION="v3.6.10"
ETCD_TARBALL="etcd-${ETCD_VERSION}-linux-amd64.tar.gz"
ETCD_URL="https://storage.googleapis.com/etcd/${ETCD_VERSION}/${ETCD_TARBALL}"
ETCD_SHA256="ed579fafab5701e3aaa95509969e7bc74776a4ae5269d32e3928408b406456ec"

usage() {
    cat <<'USAGE'
Usage: install-control-plane.sh (--build | --use-prebuilt --bin-dir <path>) [options]

Options:
  --build                 Build Tugboat binaries with cargo.
  --use-prebuilt          Use binaries from --bin-dir.
  --bin-dir <path>        Directory containing prebuilt Tugboat binaries.
  --listen <addr:port>    API server HTTP listen address. Default: 0.0.0.0:8080.
  --secure                Enable HTTPS with a locally generated Tugboat CA. Default listen: 0.0.0.0:8443.
  --pki-dir <path>        PKI directory used with --secure. Default: /etc/tugboat/pki.
  --apiserver-host <name> DNS SAN for the apiserver certificate. May be repeated.
  --apiserver-ip <addr>   IP SAN for the apiserver certificate. May be repeated.
  --force-pki             Regenerate existing PKI when used with --secure.
  --etcd-listen <addr:port>
                          etcd client listen address. Default: 127.0.0.1:2379.
  --data-dir <path>       etcd data directory. Default: /var/lib/tugboat-etcd.
USAGE
}

strip_scheme() {
    local value="$1"

    value="${value#http://}"
    value="${value#https://}"
    printf '%s\n' "${value}"
}

health_url() {
    local listen_addr="$1"
    local scheme="$2"
    local host="${listen_addr%:*}"
    local port="${listen_addr##*:}"

    if [[ "${host}" == "0.0.0.0" ]]; then
        host="127.0.0.1"
    fi

    printf '%s://%s:%s/healthz\n' "${scheme}" "${host}" "${port}"
}

apiserver_client_url() {
    local listen_addr="$1"
    local scheme="$2"
    local host="${listen_addr%:*}"
    local port="${listen_addr##*:}"

    if [[ "${scheme}" == "https" && ( "${host}" == "0.0.0.0" || "${host}" == "::" ) ]]; then
        host="localhost"
    fi

    printf '%s://%s:%s\n' "${scheme}" "${host}" "${port}"
}

is_ipv4_address() {
    [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]
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

parse_args() {
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
            --listen)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--listen requires an address."
                    return 2
                fi
                APISERVER_LISTEN="$(strip_scheme "$2")"
                LISTEN_SET=1
                shift 2
                ;;
            --listen=*)
                APISERVER_LISTEN="$(strip_scheme "${1#--listen=}")"
                LISTEN_SET=1
                shift
                ;;
            --secure)
                SECURE=1
                shift
                ;;
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
                APISERVER_CERT_HOSTS+=("$2")
                shift 2
                ;;
            --apiserver-host=*)
                if [[ -z "${1#--apiserver-host=}" ]]; then
                    log_error "--apiserver-host requires a name."
                    return 2
                fi
                APISERVER_CERT_HOSTS+=("${1#--apiserver-host=}")
                shift
                ;;
            --apiserver-ip)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--apiserver-ip requires an address."
                    return 2
                fi
                APISERVER_CERT_IPS+=("$2")
                shift 2
                ;;
            --apiserver-ip=*)
                if [[ -z "${1#--apiserver-ip=}" ]]; then
                    log_error "--apiserver-ip requires an address."
                    return 2
                fi
                APISERVER_CERT_IPS+=("${1#--apiserver-ip=}")
                shift
                ;;
            --force-pki)
                FORCE_PKI=1
                shift
                ;;
            --etcd-listen)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--etcd-listen requires an address."
                    return 2
                fi
                ETCD_LISTEN="$(strip_scheme "$2")"
                shift 2
                ;;
            --etcd-listen=*)
                ETCD_LISTEN="$(strip_scheme "${1#--etcd-listen=}")"
                shift
                ;;
            --data-dir)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--data-dir requires a path."
                    return 2
                fi
                DATA_DIR="$2"
                shift 2
                ;;
            --data-dir=*)
                DATA_DIR="${1#--data-dir=}"
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

install_base_packages() {
    apt-get update
    apt-get install -y ca-certificates curl gettext-base openssl tar wget
}

configure_tls() {
    local listen_host="${APISERVER_LISTEN%:*}"
    local setup_args=()
    local value

    APISERVER_TLS_CONFIG=""
    APISERVER_CLIENT_TLS_CONFIG=""
    APISERVER_SCHEME="http"

    if [[ "${SECURE}" -ne 1 ]]; then
        export APISERVER_TLS_CONFIG APISERVER_CLIENT_TLS_CONFIG APISERVER_SCHEME
        return 0
    fi

    APISERVER_SCHEME="https"
    if [[ "${LISTEN_SET}" -ne 1 ]]; then
        APISERVER_LISTEN="0.0.0.0:8443"
        listen_host="0.0.0.0"
    fi

    append_unique APISERVER_CERT_HOSTS control-plane
    if [[ "${listen_host}" != "0.0.0.0" && "${listen_host}" != "::" ]]; then
        if is_ipv4_address "${listen_host}"; then
            append_unique APISERVER_CERT_IPS "${listen_host}"
        else
            append_unique APISERVER_CERT_HOSTS "${listen_host}"
        fi
    fi

    setup_args=(--pki-dir "${PKI_DIR}")
    for value in "${APISERVER_CERT_HOSTS[@]}"; do
        setup_args+=(--apiserver-host "${value}")
    done
    for value in "${APISERVER_CERT_IPS[@]}"; do
        setup_args+=(--apiserver-ip "${value}")
    done
    if [[ "${FORCE_PKI}" -eq 1 ]]; then
        setup_args+=(--force)
    fi

    "${INSTALLER_DIR}/setup-pki.sh" "${setup_args[@]}"
    chown root:tugboat -- "${PKI_DIR}/apiserver.key"
    chmod 0640 -- "${PKI_DIR}/apiserver.key"

    APISERVER_TLS_CONFIG="$(
        printf '[http.tls]\ncert_file = "%s/apiserver.crt"\nkey_file = "%s/apiserver.key"' \
            "${PKI_DIR}" \
            "${PKI_DIR}"
    )"
    APISERVER_CLIENT_TLS_CONFIG="$(
        printf '[apiserver.tls]\nca_cert_path = "%s/ca.crt"' \
            "${PKI_DIR}"
    )"
    export APISERVER_TLS_CONFIG APISERVER_CLIENT_TLS_CONFIG APISERVER_SCHEME
}

install_etcd_from_package() {
    if ! apt-get install -y etcd-server; then
        return 1
    fi

    if ! command -v etcd >/dev/null 2>&1; then
        log_warn "etcd-server package installed, but etcd was not found on PATH."
        return 1
    fi

    install -m 0755 -- "$(command -v etcd)" /usr/local/bin/etcd
    if command -v etcdctl >/dev/null 2>&1; then
        install -m 0755 -- "$(command -v etcdctl)" /usr/local/bin/etcdctl
    fi
}

install_etcd_from_upstream() {
    local dest_dir
    local extracted_dir

    dest_dir="$(mktemp -d -t tugboat-etcd.XXXXXXXXXX)"
    trap 'rm -rf -- "${dest_dir}"' RETURN

    fetch_tarball "${ETCD_URL}" "${ETCD_SHA256}" "${dest_dir}"
    extracted_dir="${dest_dir}/etcd-${ETCD_VERSION}-linux-amd64"

    install -m 0755 -- "${extracted_dir}/etcd" /usr/local/bin/etcd
    install -m 0755 -- "${extracted_dir}/etcdctl" /usr/local/bin/etcdctl
    install -m 0755 -- "${extracted_dir}/etcdutl" /usr/local/bin/etcdutl
}

install_etcd() {
    if install_etcd_from_package; then
        log_info "Installed etcd from OS package."
        return 0
    fi

    log_warn "Falling back to upstream etcd ${ETCD_VERSION} tarball."
    install_etcd_from_upstream
}

render_configs() {
    install -d -m 0755 -- \
        /etc/tugboat/apiserver \
        /etc/tugboat/scheduler \
        /etc/tugboat/controller-manager

    render_template \
        "${INSTALLER_DIR}/configs/apiserver.config.toml.tpl" \
        /etc/tugboat/apiserver/config.toml
    render_template \
        "${INSTALLER_DIR}/configs/scheduler.config.toml.tpl" \
        /etc/tugboat/scheduler/config.toml
    render_template \
        "${INSTALLER_DIR}/configs/controller-manager.config.toml.tpl" \
        /etc/tugboat/controller-manager/config.toml
}

install_units() {
    install_unit "${INSTALLER_DIR}/units/etcd.service.tpl" etcd.service
    install_unit "${INSTALLER_DIR}/units/tugboat-apiserver.service.tpl" tugboat-apiserver.service
    install_unit "${INSTALLER_DIR}/units/tugboat-scheduler.service.tpl" tugboat-scheduler.service
    install_unit \
        "${INSTALLER_DIR}/units/tugboat-controller-manager.service.tpl" \
        tugboat-controller-manager.service
}

wait_for_apiserver() {
    local url
    local last_status=""
    local curl_args=()

    url="$(health_url "${APISERVER_LISTEN}" "${APISERVER_SCHEME}")"
    if [[ "${SECURE}" -eq 1 ]]; then
        curl_args=(--cacert "${PKI_DIR}/ca.crt")
    fi
    for _ in {1..60}; do
        if curl -sf "${curl_args[@]}" "${url}" >/dev/null; then
            log_info "API server is healthy: ${url}"
            return 0
        fi
        last_status="$(systemctl is-active tugboat-apiserver.service 2>/dev/null || true)"
        sleep 2
    done

    log_error "API server did not become healthy at ${url}; service state: ${last_status}"
    journalctl --no-pager -u etcd.service -u tugboat-apiserver.service || true
    return 1
}

main() {
    require_root
    parse_build_mode "$@"
    parse_args "$@"

    APISERVER_LISTEN="$(strip_scheme "${APISERVER_LISTEN}")"
    ETCD_LISTEN="$(strip_scheme "${ETCD_LISTEN}")"
    if [[ "${SECURE}" -eq 1 && "${LISTEN_SET}" -ne 1 ]]; then
        APISERVER_LISTEN="0.0.0.0:8443"
    fi
    APISERVER_SCHEME="http"
    if [[ "${SECURE}" -eq 1 ]]; then
        APISERVER_SCHEME="https"
    fi
    APISERVER_URL="$(apiserver_client_url "${APISERVER_LISTEN}" "${APISERVER_SCHEME}")"
    ETCD_ENDPOINT="http://${ETCD_LISTEN}"
    APISERVER_TLS_CONFIG=""
    APISERVER_CLIENT_TLS_CONFIG=""
    export APISERVER_LISTEN APISERVER_URL ETCD_LISTEN ETCD_ENDPOINT DATA_DIR APISERVER_TLS_CONFIG APISERVER_CLIENT_TLS_CONFIG

    install_base_packages
    install_etcd
    create_system_user tugboat
    create_system_user tugboat-etcd
    configure_tls
    APISERVER_URL="$(apiserver_client_url "${APISERVER_LISTEN}" "${APISERVER_SCHEME}")"
    export APISERVER_LISTEN APISERVER_URL
    install -d -m 0700 -o tugboat-etcd -- "${DATA_DIR}"
    install -d -m 0755 -o tugboat -- /var/log/tugboat

    install_binary tugboat-apiserver /usr/local/bin
    install_binary tugboat-scheduler /usr/local/bin
    install_binary tugboat-controller-manager /usr/local/bin

    render_configs
    install_units

    enable_unit etcd.service
    enable_unit tugboat-apiserver.service
    enable_unit tugboat-scheduler.service
    enable_unit tugboat-controller-manager.service

    wait_for_apiserver
}

main "$@"
