# Tugboat Ansible Installer

This directory contains the Ansible entry point for installing Tugboat on
systemd-based hosts. The first implementation is intentionally shaped around the
existing `installer/systemd` scripts, and later roles will replace shell-backed
steps where native Ansible tasks are safer.

## Requirements

- Ansible Core 2.16 or newer on the control machine.
- Ubuntu 24.04 targets using systemd.
- SSH access to each target with privilege escalation to root.
- Either a Rust build environment on targets when `tugboat_build_mode: build`,
  or prebuilt Tugboat binaries when `tugboat_build_mode: prebuilt`.

Run examples from the repository root:

```bash
cd installer/ansible
ansible-playbook --syntax-check -i inventory.example.yml site.yml
ansible-playbook -i inventory.example.yml site.yml
```

## Test

The Ansible installer reuses the Docker systemd environment from
`installer/systemd/test`. The containers run Ubuntu 24.04 with systemd as PID 1
and execute `ansible-playbook` inside the target container with a local
connection.

Run every Ansible installer scenario:

```bash
installer/ansible/test/run-tests.sh
```

Run one scenario:

```bash
installer/ansible/test/run-tests.sh --scenario 01-control-plane-only
```

Keep containers and volumes for debugging:

```bash
installer/ansible/test/run-tests.sh --scenario 02-worker-join --keep
```

Clean up manually after a kept run:

```bash
docker compose -f installer/systemd/test/docker-compose.test.yml -p tugboat-ansible-test down -v
```

The runner performs shell checks, `ansible-playbook --syntax-check` for
`site.yml` and `uninstall.yml`, then runs the Docker scenarios. Scenario logs and
service journals are collected under `installer/ansible/test/artifacts/` on
failure. CI currently runs the syntax checks only; privileged Docker scenarios
remain a local validation path.

Covered scenarios map to the systemd installer names:

- `01-control-plane-only`
- `02-worker-join`
- `05-tls-pki`
- `06-serviceaccount-rbac`
- `07-etcd-client-tls`
- `09-flanneld-daemon`
- `10-csi-hostpath`
- `11-idempotency`

## Inventory Model

The public inventory groups are:

- `tugboat_control_plane`: hosts that run etcd, the API server, scheduler, and
  controller manager.
- `tugboat_workers`: hosts that run the Tugboat agent and VM runtime.
- `tugboat_csi_hostpath`: hosts that run the CSI hostpath provisioner through
  the `tugboat_csi_hostpath` role.

A single host can be in multiple groups. Worker-only hosts can join an existing
API server by setting `tugboat_apiserver_advertise_url` and omitting the host
from `tugboat_control_plane`.

## Single-Node Example

Use the same host as the control plane, worker, and CSI hostpath node:

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

Install it:

```bash
cd installer/ansible
ansible-playbook -i inventory.example.yml site.yml
```

## Multi-Node Example

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

## Three-Node etcd/Control Plane Example

```yaml
all:
  vars:
    ansible_user: ubuntu
    tugboat_etcd_initial_cluster: cp-1=https://192.0.2.10:2380,cp-2=https://192.0.2.11:2380,cp-3=https://192.0.2.12:2380
    tugboat_etcd_initial_cluster_state: new
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

## Worker-Only Join Example

Use this shape when the API server already exists and the inventory should only
install worker services:

```yaml
all:
  vars:
    ansible_user: ubuntu
    tugboat_apiserver_advertise_url: https://192.0.2.10:8443
    tugboat_ca_cert: /etc/tugboat/pki/ca.crt
  children:
    tugboat_workers:
      hosts:
        worker-1:
          ansible_host: 192.0.2.20
          tugboat_node_name: worker-1
```

## Secure and Insecure Modes

Secure mode is the default:

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

Insecure mode maps to the `--insecure` systemd installer option:

```yaml
tugboat_secure: false
tugboat_apiserver_listen: 0.0.0.0:8080
tugboat_apiserver_advertise_url: http://192.0.2.10:8080
```

## Build and Prebuilt Modes

Build Tugboat binaries on each target:

```yaml
tugboat_build_mode: build
tugboat_cargo_manifest_dir: /workspace
```

Use prebuilt binaries from a directory on each target:

```yaml
tugboat_build_mode: prebuilt
tugboat_prebuilt_bin_dir: /opt/tugboat/bin
```

These map to `--build` or `--use-prebuilt --bin-dir` in
`install-control-plane.sh` and `install-worker.sh`.
The shell-backed control-plane role runs on the target, so build mode requires
the Tugboat repository to exist at `tugboat_cargo_manifest_dir` on that target.

## Worker and Flannel Settings

```yaml
tugboat_worker_runtime: qemu
tugboat_flannel_mode: static
tugboat_cni_subnet: 10.244.0.0/16
```

`tugboat_worker_runtime` maps to `--runtime` and accepts `qemu` or
`cloud-hypervisor`. `tugboat_flannel_mode` maps to `--flannel-mode` and accepts
`static`, `vxlan`, or `host-gw`.

## CSI Hostpath

```yaml
tugboat_csi_hostpath_enabled: true
tugboat_csi_hostpath_install_mode: build
tugboat_csi_hostpath_version: v1.17.0
tugboat_csi_hostpath_data_dir: /var/lib/tugboat-csi-hostpath
```

To install an existing `hostpathplugin` binary:

```yaml
tugboat_csi_hostpath_install_mode: binary
tugboat_csi_hostpath_binary: /opt/tugboat/bin/hostpathplugin
```

## Uninstall and Purge

Run uninstall against the same inventory:

```bash
cd installer/ansible
ansible-playbook -i inventory.example.yml uninstall.yml
```

Remove only worker components from a worker-only inventory:

```yaml
tugboat_uninstall_control_plane: false
tugboat_uninstall_worker: true
tugboat_uninstall_csi_hostpath: false
tugboat_uninstall_purge: false
```

Remove only the CSI hostpath provisioner from a CSI inventory:

```yaml
tugboat_uninstall_control_plane: false
tugboat_uninstall_worker: false
tugboat_uninstall_csi_hostpath: true
tugboat_uninstall_purge: false
```

Purge data directories as well:

```bash
cd installer/ansible
ansible-playbook -i inventory.example.yml uninstall.yml -e tugboat_uninstall_purge=true
```

The uninstall variables map to `uninstall.sh --control-plane`, `--worker`,
`--csi-hostpath`, and `--purge`.

## Variable Mapping

The most important direct mappings to `installer/systemd` are:

- `tugboat_build_mode`: `--build` or `--use-prebuilt`.
- `tugboat_prebuilt_bin_dir`: `--bin-dir`.
- `tugboat_apiserver_listen`: `install-control-plane.sh --listen`.
- `tugboat_apiserver_advertise_url`: `install-worker.sh --apiserver-url`.
- `tugboat_secure`: `--secure` or `--insecure`.
- `tugboat_pki_dir`: `install-control-plane.sh --pki-dir`.
- `tugboat_apiserver_cert_hosts`: repeated `--apiserver-host`.
- `tugboat_apiserver_cert_ips`: repeated `--apiserver-ip`.
- `tugboat_force_pki`: `--force-pki`.
- `tugboat_etcd_*`: the matching `install-control-plane.sh --etcd-*` options.
- `tugboat_etcd_server_cert_*`: repeated `setup-etcd-pki.sh --server-*`.
- `tugboat_etcd_peer_cert_*`: repeated `setup-etcd-pki.sh --peer-*`.
- `tugboat_node_name`: `install-worker.sh --node-name`.
- `tugboat_worker_runtime`: `install-worker.sh --runtime`.
- `tugboat_cni_subnet`: `install-worker.sh --cni-subnet`.
- `tugboat_flannel_mode`: `install-worker.sh --flannel-mode`.
- `tugboat_csi_hostpath_*`: the matching `install-csi-hostpath.sh` options.
- `tugboat_uninstall_*`: the matching `uninstall.sh` component and purge
  options.
