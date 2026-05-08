#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

APISERVER_URL=""
CA_CERT=""
PKI_DIR="/etc/tugboat/pki"
NODE_NAME="$(hostname -s)"
RUNTIME="qemu"
CNI_SUBNET="10.244.0.0/16"
FLANNEL_MODE="static"
FLANNEL_ETCD_ENDPOINTS=""
FLANNEL_ETCD_PREFIX="/coreos.com/network"
FLANNEL_ETCD_CA=""
FLANNEL_ETCD_CERT=""
FLANNEL_ETCD_KEY=""
RUNTIME_BINARY="tugboat-qemu-runtime"
RUNTIME_CONFIG_FILE="config.toml"
SECURE=0
SERVICE_ACCOUNT_TOKEN_SOURCE=""
SERVICE_ACCOUNT_TOKEN_PATH="/var/run/secrets/tugboat.cloud/serviceaccount/agent/token"

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
  --ca-cert <path>                CA certificate for https apiserver URLs. Required for https.
  --secure                        Use a ServiceAccount token for apiserver authentication.
  --service-account-token <path>  Token file to install when --secure is set.
  --node-name <name>              Tugboat node name. Default: hostname -s.
  --runtime <qemu|cloud-hypervisor>
                                  VM runtime. Default: qemu.
  --cni-subnet <cidr>             CIDR written to /run/flannel/subnet.env. Default: 10.244.0.0/16.
  --flannel-mode <static|vxlan|host-gw>
                                  static writes subnet.env; vxlan/host-gw run flanneld. Default: static.
  --flannel-etcd-endpoints <urls> Comma-separated etcd endpoints for flanneld.
  --flannel-etcd-ca <path>        etcd TLS CA certificate for flanneld.
  --flannel-etcd-cert <path>      etcd TLS client certificate for flanneld.
  --flannel-etcd-key <path>       etcd TLS client private key for flanneld.
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
            --ca-cert)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--ca-cert requires a path."
                    return 2
                fi
                CA_CERT="$2"
                shift 2
                ;;
            --ca-cert=*)
                CA_CERT="${1#--ca-cert=}"
                if [[ -z "${CA_CERT}" ]]; then
                    log_error "--ca-cert requires a path."
                    return 2
                fi
                shift
                ;;
            --secure)
                SECURE=1
                shift
                ;;
            --service-account-token)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--service-account-token requires a path."
                    return 2
                fi
                SERVICE_ACCOUNT_TOKEN_SOURCE="$2"
                shift 2
                ;;
            --service-account-token=*)
                SERVICE_ACCOUNT_TOKEN_SOURCE="${1#--service-account-token=}"
                if [[ -z "${SERVICE_ACCOUNT_TOKEN_SOURCE}" ]]; then
                    log_error "--service-account-token requires a path."
                    return 2
                fi
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
            --flannel-mode)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--flannel-mode requires static, vxlan, or host-gw."
                    return 2
                fi
                FLANNEL_MODE="$2"
                shift 2
                ;;
            --flannel-mode=*)
                FLANNEL_MODE="${1#--flannel-mode=}"
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

    if [[ -z "${APISERVER_URL}" ]]; then
        log_error "--apiserver-url is required."
        usage >&2
        return 2
    fi

    if [[ "${APISERVER_URL}" == https://* && -z "${CA_CERT}" ]]; then
        log_error "--ca-cert is required when --apiserver-url uses https."
        return 2
    fi
    if [[ "${APISERVER_URL}" != https://* && -n "${CA_CERT}" ]]; then
        log_error "--ca-cert can only be used when --apiserver-url uses https."
        return 2
    fi
    if [[ "${SECURE}" -eq 1 && -z "${SERVICE_ACCOUNT_TOKEN_SOURCE}" && ! -f "${SERVICE_ACCOUNT_TOKEN_PATH}" ]]; then
        log_error "--secure requires --service-account-token unless ${SERVICE_ACCOUNT_TOKEN_PATH} already exists."
        return 2
    fi
    if [[ "${SECURE}" -ne 1 && -n "${SERVICE_ACCOUNT_TOKEN_SOURCE}" ]]; then
        log_error "--service-account-token can only be used with --secure."
        return 2
    fi

    case "${FLANNEL_MODE}" in
        static|vxlan|host-gw)
            ;;
        *)
            log_error "--flannel-mode must be static, vxlan, or host-gw."
            return 2
            ;;
    esac

    if [[ "${FLANNEL_MODE}" == "static" ]]; then
        if [[ -n "${FLANNEL_ETCD_ENDPOINTS}${FLANNEL_ETCD_CA}${FLANNEL_ETCD_CERT}${FLANNEL_ETCD_KEY}" ]]; then
            log_error "--flannel-etcd-* options can only be used with --flannel-mode vxlan or host-gw."
            return 2
        fi
    else
        if [[ -z "${FLANNEL_ETCD_ENDPOINTS}" ]]; then
            log_error "--flannel-etcd-endpoints is required when --flannel-mode is ${FLANNEL_MODE}."
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
    apt-get install -y ca-certificates curl gettext-base kmod tar wget
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

ensure_br_netfilter() {
    if [[ "${FLANNEL_MODE}" == "static" ]]; then
        return 0
    fi
    if [[ -e /proc/sys/net/bridge/bridge-nf-call-iptables ]]; then
        return 0
    fi
    if modprobe br_netfilter 2>/dev/null; then
        return 0
    fi

    log_warn "br_netfilter is not available; trying to install linux-modules-extra for the running kernel."
    if apt-get install -y "linux-modules-extra-$(uname -r)"; then
        modprobe br_netfilter 2>/dev/null || true
    fi

    if [[ ! -e /proc/sys/net/bridge/bridge-nf-call-iptables ]]; then
        log_warn "br_netfilter is still unavailable; flanneld may fail until the module is installed on the host."
    fi
}

install_service_account_token() {
    if [[ "${SECURE}" -ne 1 ]]; then
        return 0
    fi

    install -d -m 0750 -- "$(dirname -- "${SERVICE_ACCOUNT_TOKEN_PATH}")"
    if [[ -n "${SERVICE_ACCOUNT_TOKEN_SOURCE}" ]]; then
        install -m 0600 -- "${SERVICE_ACCOUNT_TOKEN_SOURCE}" "${SERVICE_ACCOUNT_TOKEN_PATH}"
    else
        chmod 0600 -- "${SERVICE_ACCOUNT_TOKEN_PATH}"
    fi
    chown root:root -- "${SERVICE_ACCOUNT_TOKEN_PATH}" "$(dirname -- "${SERVICE_ACCOUNT_TOKEN_PATH}")"
}

render_configs() {
    install -d -m 0755 -- /etc/tugboat/agent /etc/tugboat/runtime
    install -d -m 0755 -- /var/lib/tugboat-agent/images /var/lib/tugboat-agent/csi

    APISERVER_CLIENT_TLS_CONFIG=""
    if [[ -n "${CA_CERT}" ]]; then
        install -d -m 0755 -- "${PKI_DIR}"
        install -m 0644 -- "${CA_CERT}" "${PKI_DIR}/ca.crt"
        APISERVER_CLIENT_TLS_CONFIG="$(
            printf '[apiserver.tls]\nca_cert_path = "%s/ca.crt"' \
                "${PKI_DIR}"
        )"
    fi

    if [[ "${SECURE}" -eq 1 ]]; then
        AGENT_APISERVER_AUTH_CONFIG="$(
            printf '[apiserver.auth]\ntype = "service-account"\ntoken_path = "%s"' \
                "${SERVICE_ACCOUNT_TOKEN_PATH}"
        )"
    else
        AGENT_APISERVER_AUTH_CONFIG="$(
            printf '[apiserver.auth]\ntype = "anonymous"'
        )"
    fi

    export \
        APISERVER_URL \
        APISERVER_CLIENT_TLS_CONFIG \
        AGENT_APISERVER_AUTH_CONFIG \
        NODE_NAME \
        RUNTIME_BINARY \
        RUNTIME_CONFIG_FILE
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
    local AGENT_FLANNEL_UNIT_DEPENDENCIES=""
    local FLANNEL_ETCD_TLS_ARGS=""

    if [[ "${FLANNEL_MODE}" != "static" ]]; then
        AGENT_FLANNEL_UNIT_DEPENDENCIES=$'Requires=flanneld.service\nAfter=flanneld.service'
        if [[ -n "${FLANNEL_ETCD_CA}" ]]; then
            FLANNEL_ETCD_TLS_ARGS="--etcd-cafile ${FLANNEL_ETCD_CA} --etcd-certfile ${FLANNEL_ETCD_CERT} --etcd-keyfile ${FLANNEL_ETCD_KEY}"
        fi

        export \
            FLANNEL_ETCD_ENDPOINTS \
            FLANNEL_ETCD_PREFIX \
            FLANNEL_ETCD_TLS_ARGS
        install_unit "${INSTALLER_DIR}/units/flanneld.service.tpl" flanneld.service
    fi

    export AGENT_FLANNEL_UNIT_DEPENDENCIES
    install_unit "${INSTALLER_DIR}/units/tugboat-agent.service.tpl" tugboat-agent.service
}

print_registration_hint() {
    local nodes_url="${APISERVER_URL%/}/api/v1/nodes"

    log_info "To verify node registration, run:"
    if [[ -n "${CA_CERT}" ]]; then
        printf '  curl --cacert %s/ca.crt %s\n' "${PKI_DIR}" "${nodes_url}"
    else
        printf '  curl %s\n' "${nodes_url}"
    fi
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

    if [[ "${FLANNEL_MODE}" == "static" ]]; then
        install_cni_plugins "${CNI_SUBNET}" static
    else
        install_cni_plugins "${CNI_SUBNET}" dynamic
    fi
    ensure_br_netfilter
    install_service_account_token
    render_configs
    install_units

    if [[ "${FLANNEL_MODE}" == "static" ]]; then
        disable_unit flanneld.service
    else
        rm -f -- /run/flannel/subnet.env
        enable_unit flanneld.service
    fi
    enable_unit tugboat-agent.service
    print_registration_hint
}

main "$@"
