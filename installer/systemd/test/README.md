# systemd installer test environment

This directory contains the Docker-based isolation environment for
`installer/systemd` scripts. It runs Debian bookworm containers with systemd as
PID 1, so installer scripts can write units, create users, call `systemctl`, and
install packages without touching the host.

## Requirements

- Docker with `docker compose`
- Linux host with cgroup v2
- Rust toolchain on the host for `cargo build --release`

The containers are privileged because systemd needs cgroup access. VM startup is
out of scope for this environment; CI runners do not reliably expose nested KVM.

## Run

Run every scenario:

```bash
installer/systemd/test/run-tests.sh
```

Run one scenario:

```bash
installer/systemd/test/run-tests.sh --scenario 01-control-plane-only.sh
```

Keep containers for debugging:

```bash
installer/systemd/test/run-tests.sh --scenario 01-control-plane-only --keep
```

Clean up manually:

```bash
docker compose -f installer/systemd/test/docker-compose.test.yml -p tugboat-systemd-test down -v
```

## Artifacts

On failure, `run-tests.sh` still collects journal output into
`installer/systemd/test/artifacts/` before tearing the environment down.

## Current staging

Task 10 provides the test harness before the installer tasks 02-08 are complete.
Scenarios that depend on missing scripts print `skip:` and exit successfully.
Once those scripts are implemented, the same scenarios execute the real
install, health, join, bootstrap, and uninstall checks.
