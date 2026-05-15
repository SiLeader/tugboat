# Verification Guide: Running Tugboat with the Sample Manifests

This guide walks you through starting the Tugboat environment with Docker Compose, applying the
sample manifests, and verifying that every resource is working correctly — including networking,
storage provisioning, and VM scheduling. It is written for engineers who are new to the project.

---

## Table of Contents

1. [Background: What is Tugboat?](#1-background-what-is-tugboat)
2. [Prerequisites](#2-prerequisites)
3. [Developer Verification Gates](#3-developer-verification-gates)
4. [How the Resources Relate to Each Other](#4-how-the-resources-relate-to-each-other)
5. [Starting the Environment](#5-starting-the-environment)
6. [Applying the Sample Manifests](#6-applying-the-sample-manifests)
7. [Verifying Each Resource](#7-verifying-each-resource)
    - [7.1 Node — Agent Registration](#71-node--agent-registration)
    - [7.2 Namespace](#72-namespace)
    - [7.3 ShipClass](#73-shipclass)
    - [7.4 StorageClass](#74-storageclass)
    - [7.5 ClusterNetworkClass and NetworkClass](#75-clusternetworkclass-and-networkclass)
    - [7.6 ConfigMap](#76-configmap)
    - [7.7 Secret](#77-secret)
    - [7.8 PersistentVolumeClaim — Storage Provisioning](#78-persistentvolumeclaim--storage-provisioning)
    - [7.9 Ship — Scheduling and Runtime](#79-ship--scheduling-and-runtime)
8. [Known Limitations of the Docker Compose Environment](#8-known-limitations-of-the-docker-compose-environment)
9. [Quick Reference: systemd + HTTP Verification](#9-quick-reference-systemd--http-verification)
10. [Troubleshooting](#10-troubleshooting)

---

## 1. Background: What is Tugboat?

Tugboat is a Kubernetes-inspired VM orchestration system written in Rust. Instead of managing
containers, it manages virtual machines (VMs) using QEMU. The API is intentionally similar to
Kubernetes so that existing knowledge transfers easily.

Key concept mapping:

| Kubernetes      | Tugboat               | Description                             |
|-----------------|-----------------------|-----------------------------------------|
| Pod             | **Ship**              | The workload unit (a running VM)        |
| Node            | **Node**              | A physical/virtual host that runs Ships |
| Deployment      | **Deployment**        | A rolling-updated set of identical Ships |
| Container image | **VM image** (OCI)    | An OCI artifact containing a disk image |
| Namespace       | **Namespace**         | Scope for namespaced resources          |

The system has four main components running in Docker Compose:

| Service              | Role                                                                 |
|----------------------|----------------------------------------------------------------------|
| `apiserver`          | REST API — stores and retrieves resources via etcd                   |
| `scheduler`          | Watches for unscheduled Ships and assigns them to Nodes              |
| `controller-manager` | Provisions PersistentVolumes and manages NetworkClass status         |
| `agent`              | Runs on each Node; reconciles Ships (pulls images, starts/stops VMs) |

---

## 2. Prerequisites

Make sure the following tools are installed on your machine before you begin:

| Tool             | Minimum version | Check with                              |
|------------------|-----------------|-----------------------------------------|
| Docker           | 24+             | `docker --version`                      |
| Docker Compose   | v2 (plugin)     | `docker compose version`                |
| curl             | any             | `curl --version`                        |
| python3 + PyYAML | 3.8+            | `python3 -c "import yaml; print('ok')"` |

If PyYAML is not installed, run:

```bash
pip install pyyaml
```

---

## 3. Developer Verification Gates

Use these gates when changing Tugboat itself. The Docker Compose verification below is a functional smoke path; it does not replace the Rust and installer gates.

### PR Gate

Run this before merging any change:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo deny check
```

### Component Gates

Run the matching component gate for the boundary you changed.

API/resource/store:

```bash
cargo test -p tugboat-resources
cargo test -p tugboat-resource-store
cargo test -p tugboat-apiserver
cargo test -p tugboat-integration-tests api_discovery
cargo test -p tugboat-integration-tests resource_versioning
cargo test -p tugboat-integration-tests watch
```

Controller-manager:

```bash
cargo test -p tugboat-controller-manager
cargo test -p tugboat-integration-tests replicaset_controller_integration
cargo test -p tugboat-integration-tests deployment_controller_integration
cargo test -p tugboat-integration-tests fleet_controller_integration
```

Agent, CNI, or CSI:

```bash
cargo test -p tugboat-agent
cargo test -p tugboat-cni-operator
cargo test -p tugboat-csi-operator
cargo test -p tugboat-integration-tests full_ship_lifecycle
cargo test -p tugboat-integration-tests pv_pvc_binding
cargo test -p tugboat-integration-tests node_drain
```

Runtime:

```bash
cargo test -p tugboat-vm-runtime-interface
cargo test -p tugboat-runtime-common
cargo test -p tugboat-qemu-runtime
cargo test -p tugboat-cloud-hypervisor-runtime
cargo test -p tugboat-integration-tests live_migration_e2e
```

Installer:

```bash
installer/systemd/test/run-tests.sh --scenario 01-control-plane-only
```

### Release Gate

`cargo deny check` is a release blocker. Run the PR gate, then the fixed installer scenario set:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
cargo deny check

installer/systemd/test/run-tests.sh --scenario 00-lib-unit
installer/systemd/test/run-tests.sh --scenario 01-control-plane-only
installer/systemd/test/run-tests.sh --scenario 02-worker-join
installer/systemd/test/run-tests.sh --scenario 06-serviceaccount-rbac
installer/systemd/test/run-tests.sh --scenario 08-etcd-ha
installer/systemd/test/run-tests.sh --scenario 10-csi-hostpath
```

## 4. How the Resources Relate to Each Other

Understanding the dependency chain helps you apply and verify resources in the right order.

```
Namespace (demo)
 ├── ShipClass (small)          — defines VM hardware profile (CPU + RAM)
 ├── StorageClass (hostpath)    — defines how PersistentVolumes are provisioned
 ├── ClusterNetworkClass        — cluster-wide network (any namespace can use it)
 ├── NetworkClass               — namespace-scoped network
 ├── ConfigMap (app-config)     — non-secret configuration data
 ├── Secret (app-secret)        — sensitive data (credentials)
 ├── PersistentVolumeClaim      — storage request; controller-manager creates a PV
 └── Ship (demo-ship)           — VM workload; references all of the above
```

**Flow after `apply.sh` runs:**

```
apply.sh
  │
  ├─▶ API server stores all resources in etcd
  │
  ├─▶ scheduler detects Ship with no nodeName
  │     └─▶ runs filter/score plugins (NetworkFit, ResourceFit, TaintToleration)
  │           └─▶ writes spec.nodeName = "node1" back to the Ship
  │
  ├─▶ controller-manager detects unbound PVC
  │     └─▶ calls hostpath CSI CreateVolume
  │           └─▶ creates a PersistentVolume and binds it to the PVC
  │
  └─▶ agent on node1 detects Ship assigned to it
        └─▶ waits for PVC to be bound
              └─▶ pulls VM image → creates QEMU VM
```

---

## 5. Starting the Environment

From the repository root:

```bash
# Build images and start all services in the background
docker compose up -d

# Wait until the API server is healthy (usually 10–20 seconds)
until curl -sf http://localhost:8080/healthz; do sleep 2; done && echo "Ready"
```

To watch all service logs in real time (optional but useful):

```bash
docker compose logs -f
```

Expected output when everything is healthy:

```
tugboat-etcd-1                  Up (healthy)
tugboat-apiserver-1             Up (healthy)
tugboat-scheduler-1             Up
tugboat-controller-manager-1    Up
tugboat-agent-1                 Up
tugboat-hostpath-provisioner-1  Up
```

---

## 6. Applying the Sample Manifests

```bash
cd manifests/samples
./apply.sh
```

The script:

1. Waits for the API server to be ready
2. Converts each YAML manifest to JSON
3. `POST`s it to the correct API endpoint
4. Prints `✓ OK (201)` for each successful creation

Expected output (abbreviated):

```
✓ OK (201) → .../api/v1/namespaces               kind: Namespace / (cluster) / demo
✓ OK (201) → .../api/v1/shipclasses              kind: ShipClass / (cluster) / small
✓ OK (201) → .../api/v1/storageclasses           kind: StorageClass / (cluster) / hostpath
✓ OK (201) → .../api/v1/clusternetworkclasses    kind: ClusterNetworkClass / (cluster) / demo-network
✓ OK (201) → .../api/v1/namespaces/demo/...      kind: NetworkClass / demo / internal-network
✓ OK (201) → .../api/v1/namespaces/demo/...      kind: ConfigMap / demo / app-config
✓ OK (201) → .../api/v1/namespaces/demo/...      kind: Secret / demo / app-secret
✓ OK (201) → .../api/v1/namespaces/demo/...      kind: PersistentVolumeClaim / demo / data-disk
✓ OK (201) → .../api/v1/namespaces/demo/...      kind: Ship / demo / demo-ship
```

If any step fails, the script exits and prints the error response from the API server.

---

## 7. Verifying Each Resource

All verification uses plain `curl` against `http://localhost:8080`. The API follows REST
conventions:

- **Cluster-scoped resources**: `GET /api/v1/{plural}/{name}`
- **Namespaced resources**: `GET /api/v1/namespaces/{namespace}/{plural}/{name}`
- **Coordination group resources**: `GET /apis/coordination/v1/namespaces/{namespace}/{plural}/{name}`

### 7.1 Node — Agent Registration

When the `agent` container starts, it automatically registers the host it is running on as a
`Node` resource and reports which CNI plugins are available.

```bash
curl -s http://localhost:8080/api/v1/nodes | python3 -m json.tool
```

**What to look for:**

```json
{
  "items": [
    {
      "metadata": {
        "name": "node1"
      },
      "spec": {
        "resource": {
          "cpu": 24,
          "memory": 29012533248
        },
        "overcommit": {
          "cpuRatio": "1",
          "memoryRatio": "1"
        }
      },
      "status": {
        "conditions": [
          {
            "type": "CniReady",
            "status": "True",
            "message": "Required CNI plugins are available on this node."
          }
        ],
        "cniPlugins": [
          {
            "name": "bridge",
            "ready": true,
            "message": "Found plugin binary at '/opt/cni/bin/bridge'."
          },
          {
            "name": "loopback",
            "ready": true,
            "message": "Found plugin binary at '/opt/cni/bin/loopback'."
          },
          {
            "name": "flannel",
            "ready": false,
            "message": "Missing flannel prerequisites: ..."
          },
          {
            "name": "portmap",
            "ready": true,
            "message": "Found plugin binary at '/opt/cni/bin/portmap'."
          }
        ]
      }
    }
  ]
}
```

**✅ Healthy signs:**

- `items` contains at least one entry (agent registered successfully)
- `status.conditions[0].type` is `"CniReady"` with `"status": "True"`
- `bridge` and `loopback` plugins show `"ready": true`

> **Note:** `flannel` being `false` is expected in the Docker Compose environment because the
> Flannel binary is not installed. The sample manifests use the `bridge` plugin, so this does
> not affect normal operation.

---

### 7.2 Namespace

```bash
curl -s http://localhost:8080/api/v1/namespaces/demo | python3 -m json.tool
```

**✅ Healthy:** HTTP 200 with `"kind": "Namespace"` and `"name": "demo"`.

---

### 7.3 ShipClass

A `ShipClass` defines the hardware profile (CPU architecture, cores, RAM) that a `Ship` will
request. Think of it like a VM flavor in OpenStack.

```bash
curl -s http://localhost:8080/api/v1/shipclasses/small | python3 -m json.tool
```

**What to look for:**

```json
{
  "kind": "ShipClass",
  "metadata": {
    "name": "small"
  },
  "spec": {
    "cpu": {
      "architecture": "x86_64",
      "cores": 2
    },
    "memory": {
      "size": "4Gi"
    }
  }
}
```

**✅ Healthy:** The spec matches the values you defined. The scheduler reads this to decide
whether a node has enough free resources to run the Ship.

---

### 7.4 StorageClass

A `StorageClass` tells the controller-manager which CSI driver to use when dynamically
provisioning storage. The sample uses the `hostpath.csi.k8s.io` driver provided by the
`hostpath-provisioner` container.

```bash
curl -s http://localhost:8080/api/v1/storageclasses/hostpath | python3 -m json.tool
```

**What to look for:**

```json
{
  "kind": "StorageClass",
  "metadata": {
    "name": "hostpath"
  },
  "spec": {
    "provisioner": "hostpath.csi.k8s.io",
    "reclaimPolicy": "Delete",
    "allowVolumeExpansion": true
  }
}
```

**✅ Healthy:** `spec.provisioner` matches the CSI driver socket configured in
`sample-configs/controller-manager/config.toml`.

---

### 7.5 ClusterNetworkClass and NetworkClass

Network classes define the network configuration that Ships attach to. A
`ClusterNetworkClass` is available cluster-wide; a `NetworkClass` is scoped to a single
namespace.

```bash
# Cluster-wide network
curl -s http://localhost:8080/api/v1/clusternetworkclasses/demo-network | python3 -m json.tool

# Namespace-scoped network
curl -s http://localhost:8080/api/v1/namespaces/demo/networkclasses/internal-network | python3 -m json.tool
```

**What to look for in ClusterNetworkClass:**

```json
{
  "kind": "ClusterNetworkClass",
  "metadata": {
    "name": "demo-network"
  },
  "spec": {
    "subnet": "10.100.0.0/24",
    "cniPlugin": "bridge",
    "internetAccess": true,
    "clusterNetwork": false
  }
}
```

**What to look for in NetworkClass:**

```json
{
  "kind": "NetworkClass",
  "metadata": {
    "namespace": "demo",
    "name": "internal-network"
  },
  "spec": {
    "subnet": "10.200.0.0/24",
    "cniPlugin": "bridge",
    "internetAccess": false,
    "clusterNetwork": true
  }
}
```

**✅ Healthy:** Both resources exist. The `cniPlugin` values (`"bridge"`) match CNI plugins
that are `"ready": true` on `node1` — this is exactly what the scheduler's `NetworkFit`
plugin checks before assigning a Ship to a node.

---

### 7.6 ConfigMap

A `ConfigMap` stores non-sensitive configuration data that can be mounted into a VM as files.

```bash
curl -s http://localhost:8080/api/v1/namespaces/demo/configmaps/app-config | python3 -m json.tool
```

**What to look for:**

```json
{
  "kind": "ConfigMap",
  "metadata": {
    "namespace": "demo",
    "name": "app-config"
  },
  "data": {
    "app.conf": "[app]\nport = 8080\ndebug = false\n",
    "log.level": "info",
    "timezone": "Asia/Tokyo"
  }
}
```

**✅ Healthy:** `data` contains the key-value pairs you defined in the manifest.

---

### 7.7 Secret

A `Secret` stores sensitive data (passwords, tokens). The manifest uses `stringData` (plain
text); the API server stores it internally as-is.

```bash
curl -s http://localhost:8080/api/v1/namespaces/demo/secrets/app-secret | python3 -m json.tool
```

**What to look for:**

```json
{
  "kind": "Secret",
  "metadata": {
    "namespace": "demo",
    "name": "app-secret"
  },
  "stringData": {
    "username": "admin",
    "password": "s3cr3tP@ssw0rd"
  }
}
```

**✅ Healthy:** The resource exists and the `stringData` keys are present.

---

### 7.8 PersistentVolumeClaim — Storage Provisioning

A `PersistentVolumeClaim` (PVC) is a request for storage. After you create a PVC, the
`controller-manager` calls the CSI driver to provision a matching `PersistentVolume` (PV) and
binds them together.

**Check the PVC:**

```bash
curl -s http://localhost:8080/api/v1/namespaces/demo/persistentvolumeclaims/data-disk \
  | python3 -m json.tool
```

**What to look for:**

```json
{
  "kind": "PersistentVolumeClaim",
  "metadata": {
    "namespace": "demo",
    "name": "data-disk"
  },
  "spec": {
    "accessModes": [
      "ReadWriteOnce"
    ],
    "storageClassName": "hostpath",
    "volumeMode": "Block",
    "requestedCapacityBytes": 1073741824
  },
  "status": {
    "phase": "Bound",
    "volumeName": "pvc-...",
    "capacityBytes": 1073741824
  }
}
```

**Check the provisioned PV:**

```bash
curl -s http://localhost:8080/api/v1/persistentvolumes | python3 -m json.tool
```

**✅ Healthy (on a full host environment):**

- PVC `status.phase` is `"Bound"`
- `status.volumeName` contains the auto-generated PV name
- A matching PV appears in the PV list with `spec.claimRef.name = "data-disk"` and
  `status.phase = "Bound"`

> **Note for Docker Compose:** The `hostpath.csi.k8s.io` driver requires loop device support
> (`losetup`) for `Block` volumes, which is not available inside the unprivileged
> `controller-manager` container. You will see an error in `controller-manager` logs like:
> ```
> losetup -f ... failed: exit status 1
> ```
> This is expected. To observe successful provisioning, use `volumeMode: Filesystem` instead
> of `Block`, or run the agent on a real Linux host. See
> [Section 7](#7-known-limitations-of-the-docker-compose-environment) for details.

**Check controller-manager logs for provisioning activity:**

```bash
docker compose logs controller-manager | grep -E 'pvc|provision|data-disk|ERROR'
```

---

### 7.9 Ship — Scheduling and Runtime

A `Ship` is the core workload resource. After creation, two things happen automatically:

1. The **scheduler** selects a node and writes `spec.nodeName` into the Ship.
2. The **agent** on that node detects the assignment and starts the VM.

**Verify scheduling (nodeName is assigned):**

```bash
curl -s http://localhost:8080/api/v1/namespaces/demo/ships/demo-ship \
  | python3 -c "
import sys, json
d = json.load(sys.stdin)
spec = d['spec']
status = d.get('status', {})
print('nodeName  :', spec.get('nodeName', '(not yet assigned)'))
print('conditions:', status.get('conditions', []))
print('IPs       :', status.get('ips', []))
"
```

**✅ Healthy (scheduling complete):**

```
nodeName  : node1
conditions: []
IPs       : []
```

- `nodeName` is populated — the scheduler ran `NetworkFit`, `ResourceFit`, and
  `TaintToleration` filter plugins, chose `node1`, and wrote the result back.
- `conditions` and `IPs` are populated further once the VM is actually running.

**Verify that the scheduler considered this Ship (scheduler logs):**

```bash
docker compose logs scheduler | grep -E 'unscheduled|Scheduling|demo-ship|node1'
```

Expected output:

```
Found 1 unscheduled ship(s)
Scheduling ship demo/demo-ship to node node1
Successfully bound ship demo/demo-ship to node node1
```

**Verify agent activity:**

```bash
docker compose logs agent | grep -v DEBUG | tail -20
```

**Verify that the list-all endpoint returns ships across all namespaces:**

```bash
# Should return demo-ship even though it lives in the "demo" namespace
curl -s http://localhost:8080/api/v1/ships \
  | python3 -c "
import sys, json
items = json.load(sys.stdin)['items']
print(f'Total ships: {len(items)}')
for s in items:
    m = s['metadata']
    print(f'  {m[\"namespace\"]}/{m[\"name\"]}  nodeName={s[\"spec\"].get(\"nodeName\",\"(pending)\")}')
"
```

Expected output:

```
Total ships: 1
  demo/demo-ship  nodeName=node1
```

---

## 8. Known Limitations of the Docker Compose Environment

The Docker Compose setup is designed to verify the **control plane flow** end-to-end. Some
low-level operations require kernel features that are not available inside standard Docker
containers.

| Feature                       | Limitation                                                                              | Workaround                                                                                     |
|-------------------------------|-----------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------|
| **Block volume provisioning** | `losetup` (loop device) fails inside the `controller-manager` container                 | Use `volumeMode: Filesystem` in the PVC, or run on a real Linux host with loop device support  |
| **VM execution (QEMU)**       | The `agent` container requires priviledged                                              | Ships will be scheduled correctly but the VM will not start; verify scheduling with `nodeName` |
| **Flannel CNI**               | The Flannel binary is not present in the agent image                                    | Use `cniPlugin: bridge` in NetworkClass manifests (already the default in the samples)         |
| **Real VM images**            | The image `ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04` used in `08_ship.yaml` may not exist | Replace with a real OCI VM image for full end-to-end testing                                   |

Despite these limitations, the following flows are fully verifiable in Docker Compose:

- ✅ All resource CRUD operations (create, read, list, update, delete)
- ✅ Node registration and CNI plugin capability reporting
- ✅ Ship scheduling (`nodeName` assignment by the scheduler)
- ✅ Cross-namespace list-all endpoints (used internally by the scheduler)
- ✅ NetworkClass / ClusterNetworkClass assignment validation (NetworkFit plugin)
- ✅ StorageClass and PVC creation and CSI provisioning attempt
- ✅ ConfigMap and Secret storage

---

## 9. Quick Reference: systemd + HTTP Verification

These commands verify a systemd installation where the control-plane host is
available as `CP_HOST` and the API server listens on HTTP port `8080`.

```bash
export CP="http://${CP_HOST:-localhost}:8080"
```

### 9.1 Control-plane services

```bash
systemctl is-active \
  etcd \
  tugboat-apiserver \
  tugboat-scheduler \
  tugboat-controller-manager

curl -sf "$CP/healthz" && echo "OK"
```

All four services should print `active`, and the health check should print
`OK`.

### 9.2 Node registration

```bash
curl -s "$CP/api/v1/nodes" | python3 -m json.tool
```

After at least one worker joins, the response should contain one or more
entries in `items`. When Flannel is installed, the node status should include a
ready plugin entry like:

```json
{"name": "flannel", "ready": true, "message": "Found flannel plugin binary and runtime state (...)"}
```

See [Node - Agent Registration](#61-node--agent-registration)
for the detailed status fields.

### 9.3 Flannel ClusterNetworkClass

```bash
curl -s "$CP/api/v1/clusternetworkclasses/cluster-network" | python3 -m json.tool
```

The expected value is:

```json
{"cniPlugin": "flannel"}
```

### 9.4 Component logs

```bash
journalctl \
  -u tugboat-apiserver \
  -u tugboat-scheduler \
  -u tugboat-controller-manager \
  --since "5 min ago" \
  | grep -E 'ERROR|WARN'

journalctl -u tugboat-agent --since "5 min ago" | grep -E 'ERROR|WARN'
```

No output means no recent warning or error lines were found.

---

## 10. Troubleshooting

### API server is not responding

```bash
docker compose ps          # check all containers are running
docker compose logs apiserver | tail -20
```

Make sure etcd is healthy first — the API server waits for etcd before starting.

### `apply.sh` returns HTTP 409 (Conflict)

The resource already exists from a previous run. Either:

```bash
# Restart with a clean state
docker compose down
docker compose up -d
```

Or delete individual resources first:

```bash
curl -X DELETE http://localhost:8080/api/v1/namespaces/demo/ships/demo-ship
```

### Ship stays `nodeName: (not scheduled yet)` after 30 seconds

Check scheduler logs:

```bash
docker compose logs scheduler | grep -E 'ERROR|WARN|filter|Reject'
```

Common reasons:

- **NetworkFit rejection**: the `cniPlugin` in your NetworkClass is not available on the node.
  Check that `cniPlugin: bridge` (not `flannel`) is used and that node1 shows `bridge: ready=true`.
- **ResourceFit rejection**: the ShipClass requests more CPU/memory than the node has free.
  Check `node.spec.resource` vs `shipclass.spec.cpu.cores` and `shipclass.spec.memory.size`.
- **Scheduler not started**: check `docker compose logs scheduler` for startup errors.

### PVC stays unbound in Docker Compose

This is expected for `volumeMode: Block` — see
[Section 7](#7-known-limitations-of-the-docker-compose-environment).
The agent will wait for the PVC to be bound before attempting to start the VM.

### Checking all component logs at once

```bash
docker compose logs --no-log-prefix apiserver scheduler controller-manager agent \
  | grep -E 'ERROR|WARN' | grep -v 'losetup'
```

## RBAC Verification Scenarios

### 1. TokenRequest and Expiry

Verify that you can request a signed JWT token and that it expires correctly.

```bash
# Request a token with 10s expiry
TOKEN_JSON=$(curl -s -X POST -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{"spec": {"audiences": ["https://apiserver.tugboat.cloud"], "expirationSeconds": 10}}' \
  https://apiserver.tugboat.cloud/v1/namespaces/default/serviceaccounts/default/token)
TOKEN=$(echo $TOKEN_JSON | jq -r .status.token)

# Use it immediately (should work)
curl -H "Authorization: Bearer $TOKEN" https://apiserver.tugboat.cloud/v1/namespaces/default/ships

# Wait 15s and try again (should return 401)
sleep 15
curl -i -H "Authorization: Bearer $TOKEN" https://apiserver.tugboat.cloud/v1/namespaces/default/ships
```

### 2. Projected Token in Ship

Verify that a Ship can use its own Service Account token.

```bash
# Apply a Ship with default ServiceAccount
# Wait for Ship to start
# Exec into the Ship (or check agent logs)
# The token should be at /var/run/secrets/tugboat.cloud/serviceaccount/token
```

### 3. Aggregated ClusterRoles

Verify that rules from a child ClusterRole are propagated to the parent.

```bash
# 1. Create a parent ClusterRole with aggregationRule
# 2. Check rules (should be empty)
# 3. Create a child ClusterRole with matching labels
# 4. Check parent rules again (should now contain rules from child)
```

### 4. Audit Logging

Verify that requests are recorded in the audit log.

```bash
# 1. Ensure audit is enabled in apiserver config
# 2. Perform some API actions (e.g., list ships, create a namespace)
# 3. Check /var/log/tugboat/audit.log
tail -f /var/log/tugboat/audit.log | jq .
```

## 11. Advanced Verification Scenarios

These scenarios cover newer features like topology-aware scheduling and snapshots.

### 11.1 Topology Pinning

Verify that a Ship is correctly scheduled to a node matching the storage topology.

1. **Label Nodes**: Label two nodes with distinct zones.
   ```bash
   # (Pseudo-commands, use kubectl or curl to patch node labels)
   label node node-a topology.tugboat.cloud/zone=zone-a
   label node node-b topology.tugboat.cloud/zone=zone-b
   ```
2. **Create StorageClass**: Use `WaitForFirstConsumer` and restrict to `zone-a`.
   ```yaml
   apiVersion: v1
   kind: StorageClass
   metadata:
     name: topology-aware-hostpath
   spec:
     provisioner: hostpath.csi.k8s.io
     volumeBindingMode: WaitForFirstConsumer
     allowedTopologies:
       - matchLabelExpressions:
           - key: topology.tugboat.cloud/zone
             values: ["zone-a"]
   ```
3. **Create PVC and Ship**: Create a PVC using this class and a Ship referencing it.
4. **Verify**: Confirm the Ship lands on `node-a` and the provisioned PV has `nodeAffinity` for `zone-a`.

### 11.2 VolumeSnapshot Round-trip

Verify that you can capture a snapshot of a volume and restore it.

1. **Create Source**: Create a PVC, bind it to a Ship, and write some data to it.
2. **Snapshot**: Create a `VolumeSnapshot` for the PVC.
   ```yaml
   apiVersion: snapshot/v1
   kind: VolumeSnapshot
   metadata:
     name: data-snapshot
     namespace: demo
   spec:
     source:
       persistentVolumeClaimName: data-disk
   ```
3. **Wait for Ready**: Confirm `status.readyToUse` is `true`.
4. **Restore**: Create a new PVC with `dataSource` pointing to the snapshot.
5. **Verify**: Bind the new PVC to a new Ship and verify the data is present.

### 11.3 ShipSnapshot Online + Restore

Verify that you can capture the full state of a running VM and restore it.

1. **Start Ship**: Boot a Ship and ensure it's in `Running` state.
2. **Snapshot**: Create a `ShipSnapshot` with `includeVolumes: true`.
   ```yaml
   apiVersion: snapshot/v1
   kind: ShipSnapshot
   metadata:
     name: ship-full-backup
     namespace: demo
   spec:
     shipName: demo-ship
     includeVolumes: true
   ```
3. **Delete Ship**: Delete the original Ship.
4. **Restore**: Create a new Ship with `restoreFromSnapshot: ship-full-backup`.
5. **Verify**: Confirm the new Ship resumes from the snapshot state (including memory state if supported by the runtime).
