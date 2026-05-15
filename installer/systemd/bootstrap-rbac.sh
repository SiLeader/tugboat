#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

APISERVER_URL="https://localhost:8443"
CA_CERT=""
TOKEN_OUTPUT_ROOT="/var/run/secrets/tugboat.cloud/serviceaccount"
TOKEN_OWNER_GROUP="tugboat"
AUTH_TOKEN_PATH=""
USE_TOKEN_REQUEST=0

usage() {
    cat <<'USAGE'
Usage: bootstrap-rbac.sh [options]

Options:
  --apiserver-url <url>       API server URL. Default: https://localhost:8443.
  --ca-cert <path>            CA certificate for https apiserver URLs.
  --token-output-root <path>  Root directory for component token files.
                              Default: /var/run/secrets/tugboat.cloud/serviceaccount.
  --token-owner-group <group> Group that may read scheduler/controller-manager tokens.
                              Default: tugboat.
  --auth-token-path <path>    Existing bearer token used when RBAC is already enabled.
                              Defaults to the controller-manager token if it exists.
  --use-token-request         Write signed JWTs from the ServiceAccount token subresource
                              instead of legacy service-account-token Secrets.
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
                shift
                ;;
            --token-output-root)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--token-output-root requires a path."
                    return 2
                fi
                TOKEN_OUTPUT_ROOT="$2"
                shift 2
                ;;
            --token-output-root=*)
                TOKEN_OUTPUT_ROOT="${1#--token-output-root=}"
                shift
                ;;
            --token-owner-group)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--token-owner-group requires a group."
                    return 2
                fi
                TOKEN_OWNER_GROUP="$2"
                shift 2
                ;;
            --token-owner-group=*)
                TOKEN_OWNER_GROUP="${1#--token-owner-group=}"
                shift
                ;;
            --auth-token-path)
                if [[ "$#" -lt 2 || -z "$2" ]]; then
                    log_error "--auth-token-path requires a path."
                    return 2
                fi
                AUTH_TOKEN_PATH="$2"
                shift 2
                ;;
            --auth-token-path=*)
                AUTH_TOKEN_PATH="${1#--auth-token-path=}"
                shift
                ;;
            --use-token-request)
                USE_TOKEN_REQUEST=1
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

    if [[ "${APISERVER_URL}" == https://* && -z "${CA_CERT}" ]]; then
        log_error "--ca-cert is required when --apiserver-url uses https."
        return 2
    fi
}

require_python3() {
    if ! command -v python3 >/dev/null 2>&1; then
        log_error "python3 is required to bootstrap Tugboat RBAC resources."
        return 1
    fi
}

apply_rbac_resources() {
    local ca_arg=()
    local auth_arg=()
    local token_request_arg=()

    if [[ -n "${CA_CERT}" ]]; then
        ca_arg=(--ca-cert "${CA_CERT}")
    fi
    if [[ -n "${AUTH_TOKEN_PATH}" ]]; then
        auth_arg=(--auth-token-path "${AUTH_TOKEN_PATH}")
    elif [[ -f "${TOKEN_OUTPUT_ROOT}/controller-manager/token" ]]; then
        auth_arg=(--auth-token-path "${TOKEN_OUTPUT_ROOT}/controller-manager/token")
    fi
    if [[ "${USE_TOKEN_REQUEST}" -eq 1 ]]; then
        token_request_arg=(--use-token-request)
    fi

    python3 - "${APISERVER_URL}" "${ca_arg[@]}" "${auth_arg[@]}" "${token_request_arg[@]}" <<'PY'
import argparse
import base64
import json
import ssl
import sys
import time
import urllib.error
import urllib.request

SYSTEM_NAMESPACE = "tugboat-system"
SERVICE_ACCOUNTS = ("scheduler", "controller-manager", "agent")


def parse_args():
    parser = argparse.ArgumentParser()
    parser.add_argument("apiserver_url")
    parser.add_argument("--ca-cert", default=None)
    parser.add_argument("--auth-token-path", default=None)
    parser.add_argument("--use-token-request", action="store_true")
    return parser.parse_args()


ARGS = parse_args()
BASE_URL = ARGS.apiserver_url.rstrip("/")
CONTEXT = None
if BASE_URL.startswith("https://"):
    CONTEXT = ssl.create_default_context(cafile=ARGS.ca_cert)
AUTH_HEADERS = {}
if ARGS.auth_token_path:
    with open(ARGS.auth_token_path, encoding="utf-8") as token_file:
        AUTH_HEADERS["Authorization"] = f"Bearer {token_file.read().strip()}"


def request(method, path, body=None, allow_not_found=False):
    data = None
    headers = {}
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    headers.update(AUTH_HEADERS)
    req = urllib.request.Request(
        f"{BASE_URL}{path}",
        data=data,
        headers=headers,
        method=method,
    )
    try:
        with urllib.request.urlopen(req, timeout=5, context=CONTEXT) as response:
            if response.status == 204:
                return None
            payload = response.read()
            if not payload:
                return None
            return json.loads(payload)
    except urllib.error.HTTPError as exc:
        if allow_not_found and exc.code == 404:
            return None
        detail = exc.read().decode(errors="replace")
        raise SystemExit(f"{method} {path} failed with HTTP {exc.code}: {detail}") from exc


def upsert(path, collection_path, name, manifest, replace=True):
    current = request("GET", path, allow_not_found=True)
    if current is None:
        request("POST", collection_path, manifest)
        return
    if replace:
        request("PUT", path, manifest)


def metadata(name, namespace=None):
    value = {"name": name}
    if namespace is not None:
        value["namespace"] = namespace
    return value


def namespace_manifest():
    return {
        "apiVersion": "v1",
        "kind": "Namespace",
        "metadata": metadata(SYSTEM_NAMESPACE),
    }


def service_account_manifest(name):
    return {
        "apiVersion": "v1",
        "kind": "ServiceAccount",
        "metadata": metadata(name, SYSTEM_NAMESPACE),
    }


def rule(api_groups, resources, verbs, resource_names=None):
    return {
        "apiGroups": api_groups,
        "resources": resources,
        "verbs": verbs,
        "resourceNames": resource_names or [],
    }


def cluster_role_manifest(name, rules):
    return {
        "apiVersion": "authorization/v1",
        "kind": "ClusterRole",
        "metadata": metadata(name),
        "rules": rules,
    }


def cluster_role_binding_manifest(name, role, service_account):
    return {
        "apiVersion": "authorization/v1",
        "kind": "ClusterRoleBinding",
        "metadata": metadata(name),
        "roleRef": {
            "apiGroup": "authorization",
            "kind": "ClusterRole",
            "name": role,
        },
        "subjects": [
            {
                "kind": "ServiceAccount",
                "apiGroup": "authorization",
                "namespace": SYSTEM_NAMESPACE,
                "name": service_account,
            }
        ],
    }


def cluster_roles():
    read = ["get", "list", "watch"]
    write = ["create", "get", "list", "watch", "update", "patch", "delete"]
    status = ["patch", "update"]
    return {
        "system-scheduler": [
            rule(["core"], [
                "nodes",
                "ships",
                "shipclasses",
                "persistentvolumeclaims",
                "persistentvolumes",
                "runtimeclasses",
                "networkclasses",
                "clusternetworkclasses",
            ], read),
            rule(["snapshot"], [
                "volumesnapshots",
                "volumesnapshotcontents",
                "volumesnapshotclasses",
                "shipsnapshots",
            ], read),
            rule(["core"], ["ships"], ["get", "list", "watch", "update", "patch"]),
            rule(["core"], ["ships/status"], status),
            rule(["coordination"], ["leases"], ["create", "get", "update", "patch"]),
        ],
        "system-controller-manager": [
            rule(["core"], ["namespaces", "nodes", "shipclasses", "runtimeclasses", "storageclasses"], read),
            rule(["core"], ["serviceaccounts", "secrets", "persistentvolumeclaims", "persistentvolumes"], write),
            rule(["core"], ["serviceaccounts/token"], ["create"]),
            rule(["core"], ["persistentvolumeclaims/status", "persistentvolumes/status"], status),
            rule(["core"], ["networkclasses", "clusternetworkclasses"], ["get", "list", "watch", "update", "patch"]),
            rule(["core"], ["networkclasses/status", "clusternetworkclasses/status"], status),
            rule(["core"], ["ships"], write),
            rule(["core"], ["ships/status"], status),
            rule(["apps"], ["deployments", "replicasets", "fleets"], write),
            rule(["apps"], ["deployments/status", "replicasets/status", "fleets/status"], status),
            rule(["snapshot"], ["volumesnapshots", "volumesnapshotcontents", "volumesnapshotclasses", "shipsnapshots"], write),
            rule(["snapshot"], ["volumesnapshots/status", "volumesnapshotcontents/status", "shipsnapshots/status"], status),
            rule(["authorization"], ["clusterroles", "clusterrolebindings"], ["create", "get", "list", "watch", "update", "patch"]),
        ],
        "system-agent": [
            rule(["core"], ["nodes"], ["create", "get", "list", "watch", "update", "patch"]),
            rule(["core"], ["nodes/status"], status),
            rule(["core"], [
                "ships",
                "shipclasses",
                "runtimeclasses",
                "networkclasses",
                "clusternetworkclasses",
                "persistentvolumeclaims",
                "persistentvolumes",
                "configmaps",
                "secrets",
            ], read),
            rule(["snapshot"], [
                "volumesnapshots",
                "volumesnapshotcontents",
                "volumesnapshotclasses",
                "shipsnapshots",
            ], read),
            rule(["snapshot"], ["shipsnapshots/status"], status),
            rule(["core"], ["ships"], ["get", "list", "watch", "patch", "update"]),
            rule(["core"], ["ships/status"], status),
            rule(["core"], ["serviceaccounts/token"], ["create"]),
        ],
    }


def wait_for_token(service_account):
    if ARGS.use_token_request:
        response = request(
            "POST",
            f"/api/v1/namespaces/{SYSTEM_NAMESPACE}/serviceaccounts/{service_account}/token",
            {
                "audiences": [BASE_URL],
                "expirationSeconds": 3600,
            },
        )
        return response["token"]

    for _ in range(60):
        service_account_obj = request(
            "GET",
            f"/api/v1/namespaces/{SYSTEM_NAMESPACE}/serviceaccounts/{service_account}",
            allow_not_found=True,
        )
        secret_refs = (service_account_obj or {}).get("secrets", [])
        for ref in secret_refs:
            name = ref.get("name")
            if not name:
                continue
            secret = request(
                "GET",
                f"/api/v1/namespaces/{SYSTEM_NAMESPACE}/secrets/{name}",
                allow_not_found=True,
            )
            if not secret or secret.get("type") != "tugboat.cloud/service-account-token":
                continue
            annotations = secret.get("metadata", {}).get("annotations", {})
            if annotations.get("tugboat.cloud/service-account.name") != service_account:
                continue
            encoded = secret.get("data", {}).get("token")
            if encoded:
                return base64.b64decode(encoded).decode()
        time.sleep(1)
    raise SystemExit(f"timed out waiting for ServiceAccount token for {service_account}")


upsert("/api/v1/namespaces/tugboat-system", "/api/v1/namespaces", SYSTEM_NAMESPACE, namespace_manifest(), replace=False)
for account in SERVICE_ACCOUNTS:
    upsert(
        f"/api/v1/namespaces/{SYSTEM_NAMESPACE}/serviceaccounts/{account}",
        f"/api/v1/namespaces/{SYSTEM_NAMESPACE}/serviceaccounts",
        account,
        service_account_manifest(account),
        replace=False,
    )

tokens = {account: wait_for_token(account) for account in SERVICE_ACCOUNTS}

for role_name, rules in cluster_roles().items():
    upsert(
        f"/apis/authorization/v1/clusterroles/{role_name}",
        "/apis/authorization/v1/clusterroles",
        role_name,
        cluster_role_manifest(role_name, rules),
    )

bindings = {
    "system-scheduler": "scheduler",
    "system-controller-manager": "controller-manager",
    "system-agent": "agent",
}
for role_name, account in bindings.items():
    binding_name = f"{role_name}-binding"
    upsert(
        f"/apis/authorization/v1/clusterrolebindings/{binding_name}",
        "/apis/authorization/v1/clusterrolebindings",
        binding_name,
        cluster_role_binding_manifest(binding_name, role_name, account),
    )

print(json.dumps(tokens))
PY
}

install_token() {
    local component="$1"
    local token="$2"
    local owner="$3"
    local group="$4"
    local mode="$5"
    local dir="${TOKEN_OUTPUT_ROOT}/${component}"
    local token_file="${dir}/token"

    install -d -m 0750 -- "${dir}"
    printf '%s\n' "${token}" > "${token_file}"
    chown "${owner}:${group}" "${token_file}" "${dir}"
    chmod "${mode}" "${token_file}"
}

write_tokens() {
    local tokens_json="$1"
    local scheduler_token
    local controller_manager_token
    local agent_token
    local token_parent

    scheduler_token="$(python3 -c 'import json, sys; print(json.load(sys.stdin)["scheduler"])' <<< "${tokens_json}")"
    controller_manager_token="$(python3 -c 'import json, sys; print(json.load(sys.stdin)["controller-manager"])' <<< "${tokens_json}")"
    agent_token="$(python3 -c 'import json, sys; print(json.load(sys.stdin)["agent"])' <<< "${tokens_json}")"

    token_parent="$(dirname -- "${TOKEN_OUTPUT_ROOT}")"
    install -d -m 0755 -- "${token_parent}"
    chown root:"${TOKEN_OWNER_GROUP}" "${token_parent}"
    install -d -m 0750 -- "${TOKEN_OUTPUT_ROOT}"
    chown root:"${TOKEN_OWNER_GROUP}" "${TOKEN_OUTPUT_ROOT}"
    install_token scheduler "${scheduler_token}" root "${TOKEN_OWNER_GROUP}" 0640
    install_token controller-manager "${controller_manager_token}" root "${TOKEN_OWNER_GROUP}" 0640
    install_token agent "${agent_token}" root root 0600
}

main() {
    require_root
    parse_args "$@"
    require_python3

    local tokens_json
    tokens_json="$(apply_rbac_resources)"
    write_tokens "${tokens_json}"
    log_info "Bootstrapped Tugboat ServiceAccount tokens and RBAC resources."
}

main "$@"
