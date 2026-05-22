# Tugboat Ansible Installer

This guide explains how to deploy Tugboat with the Ansible playbooks in
`installer/ansible/`. The playbooks target Ubuntu 24.04 systemd hosts and wrap
the lower-level systemd installer scripts with inventory-driven configuration,
PKI distribution, service health checks, idempotency markers, and uninstall
support.

## 1. Overview

The Ansible installer provides three install plays:

| Play | Inventory group | Installed components |
|---|---|---|
| Control plane | `tugboat_control_plane` | etcd, `tugboat-apiserver`, `tugboat-scheduler`, `tugboat-controller-manager` |
| Worker | `tugboat_workers` | `tugboat-agent`, VM runtime, CNI plugins, optional `flanneld` |
| CSI hostpath | `tugboat_csi_hostpath` | Kubernetes CSI hostpath provisioner |

The main entry points are:

| File | Purpose |
|---|---|
| `installer/ansible/site.yml` | Installs the selected control-plane, worker, and CSI hostpath roles |
| `installer/ansible/uninstall.yml` | Removes selected Tugboat components through the systemd uninstaller |
| `installer/ansible/inventory.example.yml` | Example multi-host inventory |
| `installer/ansible/group_vars/all.yml` | Default variable mapping to the systemd installer flags |

A host can be present in more than one group. For example, a single-node
cluster usually belongs to all three groups.

## 2. Requirements

| Requirement | Notes |
|---|---|
| Ansible Core 2.16+ | Required on the control machine |
| Ubuntu 24.04 targets | The playbooks use `apt` and systemd |
| SSH and privilege escalation | Target hosts must allow `become: true` |
| Rust toolchain or prebuilt binaries | `tugboat_build_mode: build` compiles on each target; `prebuilt` uses an existing binary directory |
| Internet access on targets | Needed by the wrapped installers for OS packages, etcd fallback downloads, CNI plugins, Flannel, and CSI hostpath build inputs |

Run Ansible commands from `installer/ansible/`:

```bash
cd installer/ansible
ansible-playbook --syntax-check -i inventory.example.yml site.yml
ansible-playbook -i inventory.example.yml site.yml
```

## 3. Inventory Model

The public inventory groups are:

- `tugboat_control_plane`: hosts that run etcd, the API server, scheduler, and
  controller manager.
- `tugboat_workers`: hosts that run the Tugboat agent and selected VM runtime.
- `tugboat_csi_hostpath`: hosts that run the CSI hostpath provisioner.

The default `tugboat_apiserver_advertise_url` points workers at the first
`tugboat_control_plane` host. Worker-only inventories must set
`tugboat_apiserver_advertise_url` explicitly.

### Single-node inventory

```yaml
all:
  children:
    tugboat_control_plane:
      hosts:
        localhost:
          ansible_connection: local
    tugboat_workers:
      hosts:
        localhost:
          ansible_connection: local
    tugboat_csi_hostpath:
      hosts:
        localhost:
          ansible_connection: local
```

### Multi-node inventory

```yaml
all:
  vars:
    ansible_user: ubuntu
    tugboat_apiserver_advertise_url: https://192.0.2.10:8443
  children:
    tugboat_control_plane:
      hosts:
        control-plane-1:
          ansible_host: 192.0.2.10
          tugboat_etcd_node_name: control-plane-1
          tugboat_etcd_listen: 0.0.0.0:2379
          tugboat_etcd_peer_listen: 0.0.0.0:2380
          tugboat_etcd_advertise_client_url: https://192.0.2.10:2379
          tugboat_etcd_initial_advertise_peer_url: https://192.0.2.10:2380
          tugboat_etcd_initial_cluster: control-plane-1=https://192.0.2.10:2380
    tugboat_workers:
      hosts:
        worker-1:
          ansible_host: 192.0.2.20
          tugboat_node_name: worker-1
        worker-2:
          ansible_host: 192.0.2.21
          tugboat_node_name: worker-2
    tugboat_csi_hostpath:
      hosts:
        worker-1:
        worker-2:
```

### Three-node etcd inventory

Set the same `tugboat_etcd_initial_cluster` value for every control-plane host,
then give each host its own etcd member name and advertise URLs:

```yaml
all:
  vars:
    ansible_user: ubuntu
    tugboat_etcd_initial_cluster: cp-1=https://192.0.2.10:2380,cp-2=https://192.0.2.11:2380,cp-3=https://192.0.2.12:2380
    tugboat_etcd_initial_cluster_state: new
    tugboat_etcd_endpoints:
      - https://192.0.2.10:2379
      - https://192.0.2.11:2379
      - https://192.0.2.12:2379
  children:
    tugboat_control_plane:
      hosts:
        cp-1:
          ansible_host: 192.0.2.10
          tugboat_etcd_node_name: cp-1
          tugboat_etcd_listen: 0.0.0.0:2379
          tugboat_etcd_peer_listen: 0.0.0.0:2380
          tugboat_etcd_advertise_client_url: https://192.0.2.10:2379
          tugboat_etcd_initial_advertise_peer_url: https://192.0.2.10:2380
        cp-2:
          ansible_host: 192.0.2.11
          tugboat_etcd_node_name: cp-2
          tugboat_etcd_listen: 0.0.0.0:2379
          tugboat_etcd_peer_listen: 0.0.0.0:2380
          tugboat_etcd_advertise_client_url: https://192.0.2.11:2379
          tugboat_etcd_initial_advertise_peer_url: https://192.0.2.11:2380
        cp-3:
          ansible_host: 192.0.2.12
          tugboat_etcd_node_name: cp-3
          tugboat_etcd_listen: 0.0.0.0:2379
          tugboat_etcd_peer_listen: 0.0.0.0:2380
          tugboat_etcd_advertise_client_url: https://192.0.2.12:2379
          tugboat_etcd_initial_advertise_peer_url: https://192.0.2.12:2380
```

## 4. Installation

### Secure mode

Secure mode is the default. It enables TLS for the API server and etcd, creates
ServiceAccount credentials, and configures control-plane components and workers
to use those credentials.

```yaml
tugboat_secure: true
tugboat_apiserver_listen: 0.0.0.0:8443
tugboat_apiserver_advertise_url: https://192.0.2.10:8443
tugboat_pki_dir: /etc/tugboat/pki
tugboat_apiserver_cert_hosts:
  - localhost
  - control-plane-1
tugboat_apiserver_cert_ips:
  - 127.0.0.1
  - 192.0.2.10
```

The bootstrap role generates API server and etcd PKI once on the first
control-plane host, distributes the shared PKI to all control-plane hosts, and
copies worker CA/token material from the first control-plane host when workers
are in the same inventory.

### Insecure mode

Use insecure mode only for local development or isolated tests:

```yaml
tugboat_secure: false
tugboat_apiserver_listen: 0.0.0.0:8080
tugboat_apiserver_advertise_url: http://192.0.2.10:8080
```

### Build mode

Build Tugboat binaries on each target:

```yaml
tugboat_build_mode: build
tugboat_cargo_manifest_dir: /workspace
```

`build` mode requires the Tugboat repository to exist on each target at
`tugboat_cargo_manifest_dir`.

### Prebuilt mode

Use binaries already present on each target:

```yaml
tugboat_build_mode: prebuilt
tugboat_prebuilt_bin_dir: /opt/tugboat/bin
```

The directory must contain the binaries required by the roles assigned to that
host, such as `tugboat-apiserver`, `tugboat-agent`,
`tugboat-qemu-runtime`, and `tugboat-cloud-hypervisor-runtime`.

## 5. Worker Configuration

The worker role supports QEMU and Cloud Hypervisor:

```yaml
tugboat_worker_runtime: qemu
```

or:

```yaml
tugboat_worker_runtime: cloud-hypervisor
```

Flannel can run in static, vxlan, or host-gw mode:

```yaml
tugboat_flannel_mode: static
tugboat_cni_subnet: 10.244.0.0/16
```

For dynamic Flannel modes, configure reachable etcd endpoints and client
certificates:

```yaml
tugboat_flannel_mode: vxlan
tugboat_flannel_etcd_endpoints: https://192.0.2.10:2379
tugboat_flannel_etcd_ca: /etc/tugboat/pki/etcd/ca.crt
tugboat_flannel_etcd_cert: /etc/tugboat/pki/etcd/client.crt
tugboat_flannel_etcd_key: /etc/tugboat/pki/etcd/client.key
```

When dynamic Flannel is enabled, the bootstrap role writes the Flannel network
configuration to etcd and the worker role waits for `flanneld.service`.

## 6. CSI Hostpath

Enable the CSI hostpath role by putting nodes in `tugboat_csi_hostpath` and
leaving the default enabled flag on:

```yaml
tugboat_csi_hostpath_enabled: true
tugboat_csi_hostpath_install_mode: build
tugboat_csi_hostpath_version: v1.17.0
tugboat_csi_hostpath_data_dir: /var/lib/tugboat-csi-hostpath
```

To use an existing `hostpathplugin` binary on the target:

```yaml
tugboat_csi_hostpath_install_mode: binary
tugboat_csi_hostpath_binary: /opt/tugboat/bin/hostpathplugin
```

The role waits for `hostpath-provisioner.service` and `/var/run/csi/csi.sock`.

## 7. Uninstalling

Run uninstall against the same inventory:

```bash
cd installer/ansible
ansible-playbook -i inventory.example.yml uninstall.yml
```

The default uninstall target is derived from group membership:

```yaml
tugboat_uninstall_control_plane: "{{ inventory_hostname in groups.get('tugboat_control_plane', []) }}"
tugboat_uninstall_worker: "{{ inventory_hostname in groups.get('tugboat_workers', []) }}"
tugboat_uninstall_csi_hostpath: "{{ inventory_hostname in groups.get('tugboat_csi_hostpath', []) }}"
tugboat_uninstall_purge: false
```

Purge data directories as well:

```bash
ansible-playbook -i inventory.example.yml uninstall.yml -e tugboat_uninstall_purge=true
```

## 8. Idempotency and State

The first Ansible implementation intentionally reuses the systemd shell
installers. Each install role stages those scripts under
`/var/lib/tugboat/ansible/systemd-installer`, builds the exact argument list,
and records a checksum marker under `/var/lib/tugboat/ansible/`.

The wrapped installer runs again when:

- the staged installer scripts change,
- the rendered argument checksum changes,
- required binaries, services, sockets, or health checks are missing, or
- secure material copied to a worker changes.

The roles also verify installed systemd units with `systemd-analyze verify`.

## 9. Testing

The Ansible installer has a Docker-based test runner that reuses the
`installer/systemd/test` environment. It runs shell syntax checks,
`ansible-playbook --syntax-check` for `site.yml` and `uninstall.yml`, optional
`shellcheck`, and selected end-to-end scenarios.

Run all scenarios:

```bash
installer/ansible/test/run-tests.sh
```

Run one scenario:

```bash
installer/ansible/test/run-tests.sh --scenario 01-control-plane-only
```

Keep containers for debugging:

```bash
installer/ansible/test/run-tests.sh --scenario 02-worker-join --keep
```

The runner collects service journals under `installer/ansible/test/artifacts/`
on failure. CI currently runs syntax checks; privileged Docker scenarios are a
local validation path.

Covered local scenarios:

- `01-control-plane-only`
- `02-worker-join`
- `05-tls-pki`
- `06-serviceaccount-rbac`
- `07-etcd-client-tls`
- `09-flanneld-daemon`
- `10-csi-hostpath`
- `11-idempotency`

## 10. Variable Mapping

The main variables map directly to the lower-level systemd installer options:

| Ansible variable | Systemd installer option |
|---|---|
| `tugboat_build_mode` | `--build` or `--use-prebuilt` |
| `tugboat_prebuilt_bin_dir` | `--bin-dir` |
| `tugboat_apiserver_listen` | `install-control-plane.sh --listen` |
| `tugboat_apiserver_advertise_url` | `install-worker.sh --apiserver-url` |
| `tugboat_secure` | `--secure` or `--insecure` |
| `tugboat_pki_dir` | `install-control-plane.sh --pki-dir` |
| `tugboat_apiserver_cert_hosts` | repeated `--apiserver-host` |
| `tugboat_apiserver_cert_ips` | repeated `--apiserver-ip` |
| `tugboat_force_pki` | `--force-pki` |
| `tugboat_etcd_*` | matching `install-control-plane.sh --etcd-*` options |
| `tugboat_etcd_server_cert_*` | repeated `setup-etcd-pki.sh --server-*` |
| `tugboat_etcd_peer_cert_*` | repeated `setup-etcd-pki.sh --peer-*` |
| `tugboat_node_name` | `install-worker.sh --node-name` |
| `tugboat_worker_runtime` | `install-worker.sh --runtime` |
| `tugboat_cni_subnet` | `install-worker.sh --cni-subnet` |
| `tugboat_flannel_mode` | `install-worker.sh --flannel-mode` |
| `tugboat_csi_hostpath_*` | matching `install-csi-hostpath.sh` options |
| `tugboat_uninstall_*` | matching `uninstall.sh` component and purge options |

