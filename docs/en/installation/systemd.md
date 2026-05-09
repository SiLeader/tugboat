# Tugboat systemd Installer

This guide explains how to use the installer scripts in `installer/systemd/` to deploy
Tugboat on bare-metal or virtual machines managed by **systemd** (Debian / Ubuntu).

---

## Table of Contents

1. [Overview](#1-overview)
2. [Prerequisites](#2-prerequisites)
3. [Installing the Control Plane](#3-installing-the-control-plane)
   - [3.1 Basic (secure, single node)](#31-basic-secure-single-node)
   - [3.2 Insecure mode (no TLS)](#32-insecure-mode-no-tls)
   - [3.3 Custom listen address and SAN names](#33-custom-listen-address-and-san-names)
   - [3.4 HA etcd cluster](#34-ha-etcd-cluster)
4. [Installing a Worker Node](#4-installing-a-worker-node)
   - [4.1 Basic (QEMU, static Flannel)](#41-basic-qemu-static-flannel)
   - [4.2 Cloud Hypervisor runtime](#42-cloud-hypervisor-runtime)
   - [4.3 Flannel with vxlan or host-gw](#43-flannel-with-vxlan-or-host-gw)
5. [Post-Installation Steps](#5-post-installation-steps)
   - [5.1 Bootstrap the Flannel ClusterNetworkClass](#51-bootstrap-the-flannel-clusternetworkclass)
   - [5.2 Write Flannel network config to etcd](#52-write-flannel-network-config-to-etcd)
   - [5.3 Install the CSI hostpath driver](#53-install-the-csi-hostpath-driver)
   - [5.4 Re-bootstrap RBAC](#54-re-bootstrap-rbac)
6. [Installed File Layout](#6-installed-file-layout)
7. [Uninstalling](#7-uninstalling)
8. [Troubleshooting](#8-troubleshooting)

---

## 1. Overview

The installer scripts handle:

- Downloading or building Tugboat binaries
- Installing etcd (from OS packages or upstream tarball)
- Generating a self-signed PKI (CA, server and client certificates)
- Writing TOML configuration files rendered from templates
- Creating system users and setting file permissions
- Registering and starting systemd units
- Bootstrapping RBAC (ServiceAccounts, ClusterRoles, tokens) for the secure path

The scripts must be run as **root** on a Debian or Ubuntu host.

| Script | Purpose |
|---|---|
| `install-control-plane.sh` | Installs apiserver, scheduler, controller-manager, and etcd |
| `install-worker.sh` | Installs agent, VM runtime, and CNI plugins |
| `bootstrap-flannel.sh` | Creates the flannel `ClusterNetworkClass` in the running API server |
| `bootstrap-flannel-etcd.sh` | Writes the Flannel network config key to etcd |
| `install-csi-hostpath.sh` | Installs the Kubernetes CSI hostpath driver |
| `bootstrap-rbac.sh` | (Re-)creates RBAC resources and service account tokens |
| `uninstall.sh` | Stops services and removes installed files |

---

## 2. Prerequisites

| Requirement | Notes |
|---|---|
| Debian 12 or Ubuntu 22.04+ | `apt-get` is used to install dependencies |
| Root access | All scripts call `require_root` and exit if not root |
| Cargo + Rust toolchain **or** prebuilt binaries | Use `--build` to compile; use `--use-prebuilt --bin-dir <path>` to skip compilation |
| `openssl` | Needed for PKI generation (installed automatically on Debian/Ubuntu) |
| `python3` | Needed for RBAC bootstrapping |
| Internet access | Required for downloading etcd (if not in apt), CNI plugins, and Flannel |

---

## 3. Installing the Control Plane

### 3.1 Basic (secure, single node)

The default mode generates a self-signed CA and TLS certificates. The API server listens on
`0.0.0.0:8443`.

**Build binaries with cargo:**

```bash
sudo installer/systemd/install-control-plane.sh --build
```

**Use prebuilt binaries:**

```bash
sudo installer/systemd/install-control-plane.sh \
  --use-prebuilt --bin-dir /path/to/bin
```

What the script does:

1. Installs `ca-certificates`, `curl`, `gettext-base`, `openssl`, `tar`, `wget` via apt
2. Installs etcd v3.6.10 (tries `apt-get install etcd-server` first, falls back to upstream tarball)
3. Creates system users: `tugboat`, `tugboat-etcd`, `tugboat-apiserver`, `tugboat-scheduler`, `tugboat-controller-manager`
4. Generates PKI under `/etc/tugboat/pki/` (Tugboat CA + apiserver certificate) and
   `/etc/tugboat/pki/etcd/` (etcd CA + server, peer, and client certificates)
5. Installs binaries to `/usr/local/bin/`
6. Renders config files under `/etc/tugboat/`
7. Installs and enables systemd units: `etcd.service`, `tugboat-apiserver.service`,
   `tugboat-scheduler.service`, `tugboat-controller-manager.service`
8. Waits for the API server health check to pass
9. Creates `tugboat-system` namespace, ServiceAccounts for `scheduler`, `controller-manager`,
   and `agent`, their ClusterRoles and ClusterRoleBindings, and writes service account tokens
   to `/var/run/secrets/tugboat.cloud/serviceaccount/`
10. Restarts the API server with RBAC mode enabled and reconfigures scheduler and
    controller-manager to authenticate via their service account tokens

### 3.2 Insecure mode (no TLS)

Use this only for local development or testing. The API server listens on `0.0.0.0:8080` with
no authentication.

```bash
sudo installer/systemd/install-control-plane.sh --build --insecure
```

### 3.3 Custom listen address and SAN names

To reach the API server from other machines you must add those addresses to the certificate.

```bash
sudo installer/systemd/install-control-plane.sh \
  --build \
  --listen 0.0.0.0:8443 \
  --apiserver-host cp.example.com \
  --apiserver-ip 192.168.1.10
```

Options:

| Option | Default | Description |
|---|---|---|
| `--listen <addr:port>` | `0.0.0.0:8443` (secure) / `0.0.0.0:8080` (insecure) | API server listen address |
| `--secure` / `--insecure` | `--secure` | Enable or disable TLS |
| `--pki-dir <path>` | `/etc/tugboat/pki` | PKI output directory |
| `--apiserver-host <name>` | *(hostname)* | Additional DNS SAN for the API server certificate; may be repeated |
| `--apiserver-ip <addr>` | *(none)* | Additional IP SAN; may be repeated |
| `--force-pki` | off | Regenerate existing certificates |
| `--data-dir <path>` | `/var/lib/tugboat-etcd` | etcd data directory |

### 3.4 HA etcd cluster

To form a multi-node etcd cluster, each control-plane node needs unique advertise and peer
URLs. Run `install-control-plane.sh` on each node with the full initial cluster list.

**Node 1 (new cluster):**

```bash
sudo installer/systemd/install-control-plane.sh \
  --build \
  --etcd-node-name n1 \
  --etcd-listen 0.0.0.0:2379 \
  --etcd-peer-listen 0.0.0.0:2380 \
  --etcd-advertise-client-url https://192.168.1.10:2379 \
  --etcd-initial-advertise-peer-url https://192.168.1.10:2380 \
  --etcd-initial-cluster "n1=https://192.168.1.10:2380,n2=https://192.168.1.11:2380" \
  --etcd-initial-cluster-state new
```

**Node 2 (joining existing cluster):**

```bash
sudo installer/systemd/install-control-plane.sh \
  --build \
  --etcd-node-name n2 \
  --etcd-listen 0.0.0.0:2379 \
  --etcd-peer-listen 0.0.0.0:2380 \
  --etcd-advertise-client-url https://192.168.1.11:2379 \
  --etcd-initial-advertise-peer-url https://192.168.1.11:2380 \
  --etcd-initial-cluster "n1=https://192.168.1.10:2380,n2=https://192.168.1.11:2380" \
  --etcd-initial-cluster-state existing \
  --etcd-endpoint https://192.168.1.10:2379 \
  --etcd-endpoint https://192.168.1.11:2379
```

HA etcd options:

| Option | Default | Description |
|---|---|---|
| `--etcd-node-name <name>` | `default` | etcd member name |
| `--etcd-listen <addr:port>` | `127.0.0.1:2379` | etcd client listen address |
| `--etcd-peer-listen <addr:port>` | `127.0.0.1:2380` | etcd peer listen address |
| `--etcd-advertise-client-url <url>` | `https://<etcd-listen>` | etcd client URL advertised to clients |
| `--etcd-initial-advertise-peer-url <url>` | `https://<etcd-peer-listen>` | etcd peer URL advertised to cluster members |
| `--etcd-initial-cluster <members>` | `<name>=<peer-url>` | Comma-separated `name=url` pairs for all cluster members |
| `--etcd-initial-cluster-state <new\|existing>` | `new` | `new` for a fresh cluster, `existing` when joining |
| `--etcd-endpoint <url>` | `<advertise-client-url>` | etcd endpoint written to the apiserver config; may be repeated |

---

## 4. Installing a Worker Node

Run `install-worker.sh` on every node that will host VMs. The node must be able to reach the
API server over the network.

### 4.1 Basic (QEMU, static Flannel)

**Secure API server (default):**

Copy the agent service account token from the control-plane node to the worker first:

```bash
# On the control-plane node
scp /var/run/secrets/tugboat.cloud/serviceaccount/agent/token worker:/tmp/agent-token

# On the worker node
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url https://192.168.1.10:8443 \
  --ca-cert /tmp/ca.crt \
  --service-account-token /tmp/agent-token
```

> The CA certificate can be found at `/etc/tugboat/pki/ca.crt` on the control-plane node.

**Insecure API server:**

```bash
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url http://192.168.1.10:8080 \
  --insecure
```

What the script does:

1. Installs `ca-certificates`, `curl`, `gettext-base`, `kmod`, `tar`, `wget` via apt
2. Installs QEMU dependencies: `qemu-system-x86`, `qemu-utils`, `ovmf`
3. Creates the `tugboat` system user
4. Installs `tugboat-agent` and `tugboat-qemu-runtime` to `/usr/local/bin/`
5. Installs CNI plugins (v1.9.0) and Flannel (v0.28.2) to `/opt/cni/bin/`
6. Writes `/etc/tmpfiles.d/tugboat-flannel.conf` to create `/run/flannel/subnet.env` at boot
7. Writes config files to `/etc/tugboat/agent/` and `/etc/tugboat/runtime/`
8. Installs and enables `tugboat-agent.service`

Worker options:

| Option | Default | Description |
|---|---|---|
| `--apiserver-url <url>` | *(required)* | API server URL, e.g. `https://192.168.1.10:8443` |
| `--ca-cert <path>` | *(required for https)* | CA certificate to verify the API server TLS certificate |
| `--secure` / `--insecure` | `--secure` | Use service-account token or anonymous auth |
| `--service-account-token <path>` | *(required with --secure unless token already installed)* | Agent service account token file |
| `--node-name <name>` | `hostname -s` | Node name registered in Tugboat |
| `--cni-subnet <cidr>` | `10.244.0.0/16` | Subnet written to `/run/flannel/subnet.env` |

### 4.2 Cloud Hypervisor runtime

```bash
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url https://192.168.1.10:8443 \
  --ca-cert /tmp/ca.crt \
  --service-account-token /tmp/agent-token \
  --runtime cloud-hypervisor
```

The script installs `cloud-hypervisor` (from `apt-get` or the upstream static binary) and
`tugboat-cloud-hypervisor-runtime`. The agent config is written to
`/etc/tugboat/runtime/cloud-hypervisor-config.toml`.

### 4.3 Flannel with vxlan or host-gw

When using a dynamic Flannel backend the script installs `flanneld.service` and the agent
depends on it.

```bash
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url https://192.168.1.10:8443 \
  --ca-cert /tmp/ca.crt \
  --service-account-token /tmp/agent-token \
  --flannel-mode vxlan \
  --flannel-etcd-endpoints https://192.168.1.10:2379 \
  --flannel-etcd-ca /etc/tugboat/pki/etcd/ca.crt \
  --flannel-etcd-cert /etc/tugboat/pki/etcd/client.crt \
  --flannel-etcd-key /etc/tugboat/pki/etcd/client.key
```

Flannel options:

| Option | Default | Description |
|---|---|---|
| `--flannel-mode <static\|vxlan\|host-gw>` | `static` | `static` writes `subnet.env`; `vxlan`/`host-gw` run flanneld |
| `--flannel-etcd-endpoints <urls>` | *(required for vxlan/host-gw)* | Comma-separated etcd endpoints for flanneld |
| `--flannel-etcd-ca <path>` | *(optional)* | etcd TLS CA certificate |
| `--flannel-etcd-cert <path>` | *(optional)* | etcd TLS client certificate |
| `--flannel-etcd-key <path>` | *(optional)* | etcd TLS client private key |

> `--flannel-etcd-ca`, `--flannel-etcd-cert`, and `--flannel-etcd-key` must be provided
> together or not at all.

---

## 5. Post-Installation Steps

### 5.1 Bootstrap the Flannel ClusterNetworkClass

After the control plane is running, create a `ClusterNetworkClass` that ships can reference.
Run this on any machine that can reach the API server.

```bash
installer/systemd/bootstrap-flannel.sh \
  --apiserver-url https://192.168.1.10:8443 \
  --name cluster-network \
  --subnet 10.244.0.0/16
```

Options:

| Option | Default | Description |
|---|---|---|
| `--apiserver-url <url>` | `http://127.0.0.1:8080` | API server URL |
| `--name <name>` | `cluster-network` | Name of the `ClusterNetworkClass` resource |
| `--subnet <cidr>` | `10.244.0.0/16` | Pod/VM subnet for this network class |

> The script is idempotent — it skips creation if the resource already exists.

### 5.2 Write Flannel network config to etcd

Required only when using `--flannel-mode vxlan` or `host-gw`. This writes the Flannel
configuration JSON to etcd so that flanneld can read it.

```bash
installer/systemd/bootstrap-flannel-etcd.sh \
  --backend vxlan \
  --subnet 10.244.0.0/16 \
  --flannel-etcd-endpoints https://127.0.0.1:2379
```

If the Tugboat etcd PKI files exist at `/etc/tugboat/pki/etcd/`, TLS credentials are
picked up automatically without passing `--flannel-etcd-ca`, `--flannel-etcd-cert`, and
`--flannel-etcd-key`.

Options:

| Option | Default | Description |
|---|---|---|
| `--backend <vxlan\|host-gw>` | `vxlan` | Flannel backend type |
| `--subnet <cidr>` | `10.244.0.0/16` | Flannel network CIDR |
| `--flannel-etcd-endpoints <urls>` | `https://127.0.0.1:2379` | Comma-separated etcd endpoints |
| `--flannel-etcd-ca <path>` | *(auto-detected)* | etcd TLS CA certificate |
| `--flannel-etcd-cert <path>` | *(auto-detected)* | etcd TLS client certificate |
| `--flannel-etcd-key <path>` | *(auto-detected)* | etcd TLS client private key |

### 5.3 Install the CSI hostpath driver

The CSI hostpath driver provides `PersistentVolume` provisioning backed by local host
directories. It is optional and only needed when using `StorageClass` resources with
`provisioner: hostpath.csi.k8s.io`.

```bash
# Build from source (requires Go toolchain)
sudo installer/systemd/install-csi-hostpath.sh --build

# Use an existing binary
sudo installer/systemd/install-csi-hostpath.sh --binary /path/to/hostpathplugin
```

Options:

| Option | Default | Description |
|---|---|---|
| `--build` | *(default)* | Build `hostpathplugin` from the upstream CSI source |
| `--binary <path>` | *(none)* | Install an existing prebuilt binary instead |
| `--version <version>` | `v1.17.0` | Upstream CSI hostpath driver version to build |
| `--node-id <name>` | `hostname -s` | CSI node identity |
| `--data-dir <path>` | `/var/lib/tugboat-csi-hostpath` | Volume storage directory |

### 5.4 Re-bootstrap RBAC

`install-control-plane.sh --secure` calls `bootstrap-rbac.sh` automatically. You only need
to run it manually when adding new service accounts or rotating tokens.

```bash
sudo installer/systemd/bootstrap-rbac.sh \
  --apiserver-url https://127.0.0.1:8443 \
  --ca-cert /etc/tugboat/pki/ca.crt
```

When RBAC is already enabled, pass an existing token so the script can authenticate:

```bash
sudo installer/systemd/bootstrap-rbac.sh \
  --apiserver-url https://127.0.0.1:8443 \
  --ca-cert /etc/tugboat/pki/ca.crt \
  --auth-token-path /var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token
```

Options:

| Option | Default | Description |
|---|---|---|
| `--apiserver-url <url>` | `https://localhost:8443` | API server URL |
| `--ca-cert <path>` | *(required for https)* | CA certificate |
| `--token-output-root <path>` | `/var/run/secrets/tugboat.cloud/serviceaccount` | Root directory for token files |
| `--token-owner-group <group>` | `tugboat` | Group that may read scheduler/controller-manager token files |
| `--auth-token-path <path>` | *(auto-detected)* | Existing bearer token to use when RBAC is already enabled |

---

## 6. Installed File Layout

### Control plane

```
/usr/local/bin/
  tugboat-apiserver
  tugboat-scheduler
  tugboat-controller-manager
  etcd
  etcdctl
  etcdutl

/etc/tugboat/
  apiserver/config.toml         (owner: tugboat-apiserver, mode 0600)
  scheduler/config.toml         (owner: tugboat-scheduler, mode 0600)
  controller-manager/config.toml (owner: tugboat-controller-manager, mode 0600)
  pki/
    ca.crt / ca.key             (Tugboat CA)
    apiserver.crt / apiserver.key
    etcd/
      ca.crt / ca.key           (etcd CA)
      server.crt / server.key
      peer.crt / peer.key
      client.crt / client.key

/var/lib/tugboat-etcd/          (etcd data directory)
/var/log/tugboat/               (log directories per component)

/etc/systemd/system/
  etcd.service
  tugboat-apiserver.service
  tugboat-scheduler.service
  tugboat-controller-manager.service

/var/run/secrets/tugboat.cloud/serviceaccount/
  scheduler/token
  controller-manager/token
  agent/token
```

### Worker node

```
/usr/local/bin/
  tugboat-agent
  tugboat-qemu-runtime          (or tugboat-cloud-hypervisor-runtime)
  cloud-hypervisor              (cloud-hypervisor runtime only)

/etc/tugboat/
  agent/config.toml
  runtime/config.toml           (QEMU)
  runtime/cloud-hypervisor-config.toml  (Cloud Hypervisor)
  pki/ca.crt                    (copy of the control-plane CA)

/opt/cni/bin/                   (CNI plugin binaries)
/etc/tmpfiles.d/tugboat-flannel.conf

/etc/systemd/system/
  tugboat-agent.service
  flanneld.service              (vxlan / host-gw mode only)
```

### CSI hostpath driver

```
/usr/local/bin/hostpathplugin
/var/lib/tugboat-csi-hostpath/
/var/run/csi/
/etc/systemd/system/hostpath-provisioner.service
```

---

## 7. Uninstalling

```bash
# Remove the control plane only
sudo installer/systemd/uninstall.sh --control-plane

# Remove a worker node only
sudo installer/systemd/uninstall.sh --worker

# Remove both control plane and worker
sudo installer/systemd/uninstall.sh --control-plane --worker

# Also delete etcd data, agent image cache, and CNI plugin binaries
sudo installer/systemd/uninstall.sh --control-plane --worker --purge
```

Without `--purge` the data directories (`/var/lib/tugboat-etcd`, `/var/lib/tugboat-agent`,
etc.) are left in place so that a re-installation can reuse them.

---

## 8. Troubleshooting

### Check service status

```bash
systemctl status etcd tugboat-apiserver tugboat-scheduler tugboat-controller-manager
systemctl status tugboat-agent
```

### View logs

```bash
journalctl -u tugboat-apiserver --since "5 min ago"
journalctl -u tugboat-agent -f
```

### API server health

```bash
# Secure
curl --cacert /etc/tugboat/pki/ca.crt https://localhost:8443/healthz

# Insecure
curl http://localhost:8080/healthz
```

### etcd health

```bash
ETCDCTL_API=3 etcdctl \
  --endpoints https://127.0.0.1:2379 \
  --cacert /etc/tugboat/pki/etcd/ca.crt \
  --cert /etc/tugboat/pki/etcd/client.crt \
  --key /etc/tugboat/pki/etcd/client.key \
  endpoint health
```

### Node registration

After a worker starts, verify it registered with the API server:

```bash
curl --cacert /etc/tugboat/pki/ca.crt https://localhost:8443/api/v1/nodes | python3 -m json.tool
```

### PKI issues

If a certificate was generated with wrong SANs, regenerate it with `--force-pki`:

```bash
sudo installer/systemd/install-control-plane.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-host myhost.example.com \
  --apiserver-ip 192.168.1.10 \
  --force-pki
```

Or regenerate the etcd PKI directly:

```bash
sudo installer/systemd/setup-etcd-pki.sh \
  --pki-dir /etc/tugboat/pki/etcd \
  --server-host etcd.example.com \
  --server-ip 192.168.1.10 \
  --peer-host etcd.example.com \
  --peer-ip 192.168.1.10 \
  --force
```

### Flannel subnet file missing

If `/run/flannel/subnet.env` is not created at boot, regenerate it manually:

```bash
sudo systemd-tmpfiles --create /etc/tmpfiles.d/tugboat-flannel.conf
```
