#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

APISERVER_URL="http://127.0.0.1:8080"
NETWORK_CLASS_NAME="cluster-network"
NETWORK_SUBNET="10.244.0.0/16"
FLANNEL_SUBNET_FILE="/run/flannel/subnet.env"
FLANNEL_DATA_DIR="/var/lib/cni/flannel"

usage() {
    cat <<'USAGE'
Usage: bootstrap-flannel.sh [options]

Options:
  --apiserver-url <url>  API server URL. Default: http://127.0.0.1:8080.
  --name <name>          ClusterNetworkClass name. Default: cluster-network.
  --subnet <cidr>        ClusterNetworkClass subnet. Default: 10.244.0.0/16.
USAGE
}

parse_args() {
    while [[ "$#" -gt 0 ]]; do
        case "$1" in
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
            --name)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--name requires a resource name."
                    return 2
                fi
                NETWORK_CLASS_NAME="$2"
                shift 2
                ;;
            --name=*)
                NETWORK_CLASS_NAME="${1#--name=}"
                shift
                ;;
            --subnet)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--subnet requires a CIDR."
                    return 2
                fi
                NETWORK_SUBNET="$2"
                shift 2
                ;;
            --subnet=*)
                NETWORK_SUBNET="${1#--subnet=}"
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
    if ! command -v curl >/dev/null 2>&1; then
        log_error "curl is required."
        return 1
    fi

    APISERVER_URL="${APISERVER_URL%/}"
    if [[ -z "${APISERVER_URL}" ]]; then
        log_error "--apiserver-url must not be empty."
        return 2
    fi

    if [[ ${#NETWORK_CLASS_NAME} -gt 253 ||
        ! "${NETWORK_CLASS_NAME}" =~ ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$ ]]; then
        log_error "--name must match Tugboat resource names: ^[a-z0-9]([-a-z0-9]*[a-z0-9])?$"
        return 2
    fi

    if [[ ! "${NETWORK_SUBNET}" =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}/[0-9]{1,2}$ ]]; then
        log_error "--subnet must be an IPv4 CIDR, for example 10.244.0.0/16."
        return 2
    fi
}

check_apiserver_health() {
    local health_url="${APISERVER_URL}/healthz"

    if ! curl -sf "${health_url}" >/dev/null; then
        log_error "API server is not healthy: ${health_url}"
        return 1
    fi
}

curl_status() {
    local url="$1"
    local method="${2:-GET}"

    curl -sS -o /dev/null -w "%{http_code}" -X "${method}" "${url}" || true
}

resource_exists() {
    local resource_url="$1"
    local status

    status="$(curl_status "${resource_url}")"
    case "${status}" in
        200)
            return 0
            ;;
        404)
            return 1
            ;;
        *)
            log_error "Failed to check ClusterNetworkClass '${NETWORK_CLASS_NAME}' (${status})."
            return 2
            ;;
    esac
}

create_cluster_network_class() {
    local collection_url="${APISERVER_URL}/api/v1/clusternetworkclasses"
    local response_file
    local http_status
    local payload

    payload="$(
        cat <<JSON
{
  "apiVersion": "v1",
  "kind": "ClusterNetworkClass",
  "metadata": {
    "name": "${NETWORK_CLASS_NAME}"
  },
  "spec": {
    "cniPlugin": "flannel",
    "flannel": {
      "subnetFile": "${FLANNEL_SUBNET_FILE}",
      "dataDir": "${FLANNEL_DATA_DIR}",
      "hairpinMode": true,
      "defaultGateway": true
    },
    "subnet": "${NETWORK_SUBNET}"
  }
}
JSON
    )"

    response_file="$(mktemp -t tugboat-bootstrap-flannel.XXXXXXXXXX)"

    http_status="$(
        curl -sS -o "${response_file}" -w "%{http_code}" \
            -X POST \
            -H "Content-Type: application/json" \
            -d "${payload}" \
            "${collection_url}" || true
    )"

    case "${http_status}" in
        200|201)
            log_info "Created ClusterNetworkClass '${NETWORK_CLASS_NAME}' with flannel CNI."
            rm -f -- "${response_file}"
            ;;
        409)
            log_warn "ClusterNetworkClass '${NETWORK_CLASS_NAME}' already exists. Skipping."
            rm -f -- "${response_file}"
            ;;
        *)
            log_error "Failed to create ClusterNetworkClass '${NETWORK_CLASS_NAME}' (${http_status})."
            if [[ -s "${response_file}" ]]; then
                cat "${response_file}" >&2
                printf '\n' >&2
            fi
            rm -f -- "${response_file}"
            return 1
            ;;
    esac
}

main() {
    local exists_status
    local resource_url

    parse_args "$@"
    validate_args
    check_apiserver_health

    resource_url="${APISERVER_URL}/api/v1/clusternetworkclasses/${NETWORK_CLASS_NAME}"
    if resource_exists "${resource_url}"; then
        log_warn "ClusterNetworkClass '${NETWORK_CLASS_NAME}' already exists. Skipping."
        return 0
    else
        exists_status="$?"
        if [[ "${exists_status}" -ne 1 ]]; then
            return "${exists_status}"
        fi
    fi

    create_cluster_network_class
}

main "$@"
