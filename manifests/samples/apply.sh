#!/usr/bin/env bash
# Copyright 2025- SiLeader (Cerussite).
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
#
# Script to apply all sample manifests to the tugboat API server in one shot.
#
# Usage:
#   # Docker Compose environment (default)
#   ./apply.sh
#
#   # Specify a custom API server URL
#   APISERVER_URL=http://localhost:8080 ./apply.sh
#
# Requirements:
#   - python3 and PyYAML (pip install pyyaml) must be installed
#   - curl must be installed
#   - The Docker Compose environment must be running (docker compose up -d)

set -euo pipefail

APISERVER_URL="${APISERVER_URL:-http://localhost:8080}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Helper: convert a YAML file to JSON
yaml_to_json() {
  python3 -c "
import sys, json
try:
    import yaml
except ImportError:
    print('ERROR: PyYAML not found. Install it with: pip install pyyaml', file=sys.stderr)
    sys.exit(1)
data = yaml.safe_load(sys.stdin)
json.dump(data, sys.stdout)
"
}

# Resolve the API endpoint URL from the manifest content
resolve_url() {
  local json="$1"
  python3 - "$json" "$APISERVER_URL" <<'EOF'
import sys, json

manifest = json.loads(sys.argv[1])
base_url = sys.argv[2].rstrip('/')

api_version = manifest.get('apiVersion', 'v1')
kind        = manifest.get('kind', '')
namespace   = manifest.get('metadata', {}).get('namespace', '')

# kind → (plural, api_prefix)
CLUSTER_RESOURCES = {
    'Namespace':           ('namespaces',            '/api/v1'),
    'Node':                ('nodes',                 '/api/v1'),
    'ShipClass':           ('shipclasses',           '/api/v1'),
    'StorageClass':        ('storageclasses',        '/api/v1'),
    'ClusterNetworkClass': ('clusternetworkclasses', '/api/v1'),
    'PersistentVolume':    ('persistentvolumes',     '/api/v1'),
    'CustomResourceDefinition': ('customresourcedefinitions', '/apis/apiextensions/v1'),
}
NAMESPACED_RESOURCES = {
    'Ship':                    ('ships',                    '/api/v1'),
    'NetworkClass':            ('networkclasses',           '/api/v1'),
    'ConfigMap':               ('configmaps',               '/api/v1'),
    'Secret':                  ('secrets',                  '/api/v1'),
    'PersistentVolumeClaim':   ('persistentvolumeclaims',   '/api/v1'),
    'Lease':                   ('leases',                   '/apis/coordination/v1'),
}

if kind in CLUSTER_RESOURCES:
    plural, prefix = CLUSTER_RESOURCES[kind]
    print(f'{base_url}{prefix}/{plural}')
elif kind in NAMESPACED_RESOURCES:
    plural, prefix = NAMESPACED_RESOURCES[kind]
    if not namespace:
        print(f'ERROR: {kind} requires a namespace', file=sys.stderr)
        sys.exit(1)
    print(f'{base_url}{prefix}/namespaces/{namespace}/{plural}')
else:
    annotations = manifest.get('metadata', {}).get('annotations', {})
    plural = annotations.get('tugboat.cloud/plural', '')
    if not plural:
        print(f'ERROR: unknown resource kind: {kind}; custom resources require metadata.annotations["tugboat.cloud/plural"]', file=sys.stderr)
        sys.exit(1)
    if '/' not in api_version:
        print(f'ERROR: custom resource apiVersion must be group/version, got: {api_version}', file=sys.stderr)
        sys.exit(1)
    group, version = api_version.split('/', 1)
    if namespace:
        print(f'{base_url}/apis/{group}/{version}/namespaces/{namespace}/{plural}')
    else:
        print(f'{base_url}/apis/{group}/{version}/{plural}')
EOF
}

# Apply a single manifest file
apply_manifest() {
  local file="$1"
  local filename
  filename="$(basename "$file")"

  echo "--- apply: $filename ---"

  local json
  json="$(yaml_to_json < "$file")"

  local url
  url="$(resolve_url "$json")"

  local http_status
  local response_body
  response_body="$(curl -s -o /tmp/tugboat_apply_response.json -w "%{http_code}" \
    -X POST \
    -H "Content-Type: application/json" \
    -d "$json" \
    "$url")"
  http_status="$response_body"

  local body
  body="$(cat /tmp/tugboat_apply_response.json)"

  if [[ "$http_status" -ge 200 && "$http_status" -lt 300 ]]; then
    echo "  ✓ OK ($http_status) → $url"
    echo "$body" | python3 -c "
import sys, json
try:
    d = json.load(sys.stdin)
    print('    kind:', d.get('kind',''), '/', d.get('metadata',{}).get('namespace','(cluster)'), '/', d.get('metadata',{}).get('name',''))
except Exception:
    pass
"
    maybe_wait_for_crd "$json"
  else
      echo "  ✗ FAILED ($http_status) → $url"
    echo "$body" | python3 -m json.tool --indent 2 2>/dev/null || echo "$body"
    if [[ "${IGNORE_ERRORS:-}" != "1" ]]; then
      exit 1
    fi
  fi
  echo
}

maybe_wait_for_crd() {
  local json="$1"
  local discovery_path
  discovery_path="$(python3 - "$json" <<'EOF'
import sys, json
manifest = json.loads(sys.argv[1])
if manifest.get('kind') != 'CustomResourceDefinition':
    sys.exit(0)
spec = manifest.get('spec', {})
versions = spec.get('versions', [])
names = spec.get('names', {})
if not spec.get('group') or not versions or not names.get('plural'):
    sys.exit(0)
print(f"/apis/{spec['group']}/{versions[0].get('name', 'v1')}#{names['plural']}")
EOF
)"
  if [[ -z "$discovery_path" ]]; then
    return
  fi

  local path="${discovery_path%%#*}"
  local plural="${discovery_path##*#}"
  for _ in {1..20}; do
    if curl -sf "$APISERVER_URL$path" | python3 -c "import sys, json; plural = sys.argv[1]; body = json.load(sys.stdin); sys.exit(0 if any(resource.get('name') == plural for resource in body.get('resources', [])) else 1)" "$plural" >/dev/null
    then
      echo "  ✓ CRD discovery ready ($path)"
      return
    fi
    sleep 0.05
  done

  echo "  ✗ CRD did not appear in discovery: $path"
  exit 1
}

# Wait for the API server to become ready
wait_for_apiserver() {
  local max_retries=30
  local retry=0
  echo "Waiting for API server to be ready: $APISERVER_URL ..."
  until curl -sf "$APISERVER_URL/healthz" > /dev/null 2>&1; do
    retry=$((retry + 1))
    if [[ $retry -ge $max_retries ]]; then
      echo "ERROR: API server did not become ready ($APISERVER_URL/healthz)"
      exit 1
    fi
    sleep 2
  done
  echo "  ✓ API server is ready ($APISERVER_URL)"
  echo
}

main() {
  wait_for_apiserver

  echo "=========================================="
  echo " tugboat sample manifests apply"
  echo " API server: $APISERVER_URL"
  echo "=========================================="
  echo
  echo "Apply order:"
  echo "  1. Namespace          (demo)"
  echo "  2. ShipClass          (small)"
  echo "  3. StorageClass       (hostpath)"
  echo "  4. ClusterNetworkClass (demo-network)"
  echo "  5. NetworkClass       (demo/internal-network)"
  echo "  6. ConfigMap          (demo/app-config)"
  echo "  7. Secret             (demo/app-secret)"
  echo "  8. PersistentVolumeClaim (demo/data-disk)"
  echo "  9. Ship               (demo/demo-ship)"
  echo " 10. CustomResourceDefinition (databases.example.com)"
  echo " 11. Custom Resource    (demo/demo-db)"
  echo

  # Apply files in dependency order
  for manifest in \
    "$SCRIPT_DIR/00_namespace.yaml" \
    "$SCRIPT_DIR/01_shipclass.yaml" \
    "$SCRIPT_DIR/02_storageclass.yaml" \
    "$SCRIPT_DIR/03_clusternetworkclass.yaml" \
    "$SCRIPT_DIR/04_networkclass.yaml" \
    "$SCRIPT_DIR/05_configmap.yaml" \
    "$SCRIPT_DIR/06_secret.yaml" \
    "$SCRIPT_DIR/07_pvc.yaml" \
    "$SCRIPT_DIR/07_pvc_fs.yaml" \
    "$SCRIPT_DIR/08_ship.yaml" \
    "$SCRIPT_DIR/crd_example.yaml" \
    "$SCRIPT_DIR/cr_example.yaml"
  do
    apply_manifest "$manifest"
  done

  echo "=========================================="
  echo " Done. Verify resources with:"
  echo ""
  echo "  # Check Ship status"
  echo "  curl -s $APISERVER_URL/api/v1/namespaces/demo/ships/demo-ship | python3 -m json.tool"
  echo
  echo "  # Check PVC status (provisioned by controller-manager)"
  echo "  curl -s $APISERVER_URL/api/v1/namespaces/demo/persistentvolumeclaims/data-disk | python3 -m json.tool"
  echo
  echo "  # Check Node status (registered by agent)"
  echo "  curl -s $APISERVER_URL/api/v1/nodes | python3 -m json.tool"
  echo
  echo "  # Check custom resource discovery and sample CR"
  echo "  curl -s $APISERVER_URL/apis/example.com/v1 | python3 -m json.tool"
  echo "  curl -s $APISERVER_URL/apis/example.com/v1/namespaces/demo/databases/demo-db | python3 -m json.tool"
  echo "=========================================="
}

main "$@"
