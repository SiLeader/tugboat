#!/usr/bin/env python3
"""Run an isolated Tugboat cluster on Multipass and verify it with Ansible.

The host-side controller deliberately uses only the Multipass and Ansible
command line interfaces.  Guest configuration remains in the repository's
Ansible roles; this file owns the cluster lease, generated connection data,
and the safe lifecycle around those roles.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime as dt
import errno
import fcntl
import hashlib
import ipaddress
import json
import os
import platform
import re
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
from pathlib import Path
from typing import Any, Callable, Iterator, Mapping, Sequence


SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parents[1]
ANSIBLE_ROOT = REPO_ROOT / "installer" / "ansible"
MULTIPASS_ANSIBLE_ROOT = SCRIPT_DIR / "ansible"
TEMPLATE_PATH = SCRIPT_DIR / "cloud-init.yaml.j2"
DEFAULT_CACHE_ROOT = REPO_ROOT / ".cache" / "multipass"
SCHEMA_VERSION = 1
CLUSTER_NAME_RE = re.compile(r"^[a-z](?:[a-z0-9-]*[a-z0-9])?$")
SSH_PUBLIC_KEY_RE = re.compile(r"^ssh-(?:ed25519|rsa|ecdsa-[^ ]+) [A-Za-z0-9+/=]+(?: .*)?$")
REQUIRED_BINARIES = (
    "tugboat-apiserver",
    "tugboat-controller-manager",
    "tugboat-scheduler",
    "tugboat-agent",
    "tugboat-qemu-runtime",
)

DEFAULT_CONFIG: dict[str, Any] = {
    "multipass": {
        "image": "24.04",
        "launch_timeout_seconds": 900,
        "cloud_init_timeout_seconds": 300,
        "ssh_timeout_seconds": 180,
        "build_timeout_seconds": 3600,
        "api_timeout_seconds": 300,
    },
    "workers": 2,
    "resources": {
        "control_plane": {"cpus": 4, "memory": "6G", "disk": "40G"},
        "worker": {"cpus": 2, "memory": "4G", "disk": "30G"},
    },
    "tugboat": {
        "secure": True,
        "runtime": "qemu",
        "flannel_mode": "vxlan",
        "cni_subnet": "10.244.0.0/16",
        "build_jobs": 2,
        "cargo_offline": False,
    },
    "demo": {
        "image": "ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04",
        "image_digest": "",
        "namespace": "tugboat-demo",
        "token_audience": "https://localhost:8443",
        "guest_probe": {
            "enabled": False,
            "ssh_user": "ubuntu",
            "ssh_identity_file": "",
            "ssh_host_public_key": "",
            "source_command": [
                "curl",
                "--fail",
                "--silent",
                "--show-error",
                "--connect-timeout",
                "5",
                "http://__PEER_IP__:__PEER_PORT__/healthz",
            ],
            "peer_address": "__PEER_IP__",
            "port": 8080,
        },
    },
}


class DemoError(RuntimeError):
    """A user-actionable failure in the demo controller."""


def now() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def _redact(value: str) -> str:
    for name in (
        "TUGBOAT_TOKEN",
        "KUBERNETES_TOKEN",
        "SERVICE_ACCOUNT_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
    ):
        secret = os.environ.get(name)
        if secret:
            value = value.replace(secret, "[redacted]")
    return value


def run_command(
    argv: Sequence[str],
    *,
    cwd: Path | None = None,
    timeout: float | None = None,
    env: Mapping[str, str] | None = None,
    input_text: str | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    """Run an external command without invoking a shell."""
    try:
        result = subprocess.run(
            [str(item) for item in argv],
            cwd=str(cwd) if cwd else None,
            env=dict(env) if env else None,
            input=input_text,
            text=True,
            capture_output=True,
            timeout=timeout,
            check=False,
        )
    except FileNotFoundError as exc:
        raise DemoError(f"command not found: {argv[0]}") from exc
    except subprocess.TimeoutExpired as exc:
        raise DemoError(f"command timed out: {_redact(shlex.join(map(str, argv)))}") from exc
    if check and result.returncode != 0:
        details = "\n".join(
            part for part in (_redact(result.stdout.strip()), _redact(result.stderr.strip())) if part
        )
        suffix = f"\n{details}" if details else ""
        raise DemoError(
            f"command failed ({result.returncode}): "
            f"{_redact(shlex.join(map(str, argv)))}{suffix}"
        )
    return result


class Multipass:
    def __init__(self, executable: str = "multipass") -> None:
        self.executable = executable

    def _run(
        self,
        args: Sequence[str],
        *,
        timeout: float | None = None,
        check: bool = True,
    ) -> subprocess.CompletedProcess[str]:
        return run_command([self.executable, *args], timeout=timeout, check=check)

    def list(self) -> list[dict[str, Any]]:
        result = self._run(["list", "--format", "json"])
        return parse_multipass_list(result.stdout)

    def info(self, name: str) -> dict[str, Any]:
        result = self._run(["info", "--format", "json", name])
        return parse_multipass_info(result.stdout, name)

    def launch(
        self,
        *,
        image: str,
        name: str,
        cpus: int,
        memory: str,
        disk: str,
        cloud_init: Path,
        timeout: float,
    ) -> None:
        self._run(
            [
                "launch",
                image,
                "--name",
                name,
                "--cpus",
                str(cpus),
                "--memory",
                memory,
                "--disk",
                disk,
                "--cloud-init",
                str(cloud_init),
            ],
            timeout=timeout,
        )

    def start(self, names: Sequence[str], *, timeout: float) -> None:
        if names:
            self._run(["start", *names], timeout=timeout)

    def stop(self, names: Sequence[str], *, timeout: float) -> None:
        if names:
            self._run(["stop", *names], timeout=timeout)

    def delete(self, names: Sequence[str], *, timeout: float) -> None:
        if names:
            self._run(["delete", "--purge", *names], timeout=timeout)

    def exec(self, name: str, argv: Sequence[str], *, timeout: float | None = None) -> str:
        result = self._run(["exec", name, "--", *argv], timeout=timeout)
        return result.stdout

    def version(self) -> str:
        return self._run(["version"]).stdout.strip()

    def driver(self) -> str:
        return self._run(["get", "local.driver"]).stdout.strip()

    def owner(self, name: str) -> str:
        return self.exec(name, ["sudo", "cat", "/etc/tugboat-multipass-owner"]).strip()

    def host_key(self, name: str) -> str:
        output = self.exec(
            name,
            ["sudo", "cat", "/etc/ssh/ssh_host_ed25519_key.pub"],
        )
        for line in output.splitlines():
            line = line.strip()
            if SSH_PUBLIC_KEY_RE.match(line):
                return line
        raise DemoError(f"{name}: no usable ed25519 host key was returned")


def parse_multipass_list(payload: str | Mapping[str, Any]) -> list[dict[str, Any]]:
    data: Any = json.loads(payload) if isinstance(payload, str) else payload
    if isinstance(data, dict) and isinstance(data.get("list"), list):
        values = data["list"]
    elif isinstance(data, list):
        values = data
    elif isinstance(data, dict):
        values = []
        for key, value in data.items():
            if isinstance(value, dict):
                entry = dict(value)
                entry.setdefault("name", key)
                values.append(entry)
    else:
        raise DemoError("Multipass returned an unexpected list JSON document")
    return [dict(value) for value in values if isinstance(value, dict) and value.get("name")]


def parse_multipass_info(
    payload: str | Mapping[str, Any],
    name: str,
) -> dict[str, Any]:
    data: Any = json.loads(payload) if isinstance(payload, str) else payload
    if isinstance(data, dict) and isinstance(data.get("info"), dict):
        info = data["info"]
        if name in info and isinstance(info[name], dict):
            result = dict(info[name])
            result.setdefault("name", name)
            return result
        if len(info) == 1:
            key, value = next(iter(info.items()))
            if isinstance(value, dict):
                result = dict(value)
                result.setdefault("name", key)
                return result
    if isinstance(data, dict) and isinstance(data.get(name), dict):
        result = dict(data[name])
        result.setdefault("name", name)
        return result
    if isinstance(data, dict):
        result = dict(data)
        result.setdefault("name", name)
        if "state" in result or "ipv4" in result:
            return result
    raise DemoError(f"{name}: Multipass returned an unexpected info JSON document")


def _walk_values(value: Any) -> Iterator[Any]:
    if isinstance(value, Mapping):
        for child in value.values():
            yield from _walk_values(child)
    elif isinstance(value, (list, tuple)):
        for child in value:
            yield from _walk_values(child)
    else:
        yield value


def extract_ipv4_addresses(info: Mapping[str, Any]) -> list[str]:
    addresses: list[str] = []
    for value in _walk_values(info):
        if not isinstance(value, str):
            continue
        try:
            address = ipaddress.ip_address(value)
        except ValueError:
            continue
        if isinstance(address, ipaddress.IPv4Address) and str(address) not in addresses:
            addresses.append(str(address))
    return addresses


def probe_tcp(address: str, port: int = 22, timeout: float = 1.5) -> bool:
    try:
        with socket.create_connection((address, port), timeout=timeout):
            return True
    except OSError:
        return False


def choose_management_ip(
    addresses: Sequence[str],
    cni_subnet: str,
    probe: Callable[[str], bool] | None = None,
) -> str:
    network = ipaddress.ip_network(cni_subnet, strict=True)
    candidates: list[str] = []
    for raw in addresses:
        try:
            address = ipaddress.ip_address(raw)
        except ValueError:
            continue
        if not isinstance(address, ipaddress.IPv4Address):
            continue
        if address.is_loopback or address.is_link_local or address.is_unspecified:
            continue
        if address in network:
            continue
        if str(address) not in candidates:
            candidates.append(str(address))
    if not candidates:
        raise DemoError("Multipass returned no management IPv4 address outside the CNI subnet")
    if probe is not None:
        reachable = [address for address in candidates if probe(address)]
        if reachable:
            return reachable[0]
    return candidates[0]


def parse_size(value: Any) -> int:
    if isinstance(value, bool):
        raise DemoError("resource sizes must be positive values")
    text = str(value).strip().upper()
    match = re.fullmatch(r"([1-9][0-9]*)([KMGTP]?B?)?", text)
    if not match:
        raise DemoError(f"invalid resource size: {value}")
    number = int(match.group(1))
    suffix = (match.group(2) or "").rstrip("B")
    return number * {
        "": 1,
        "K": 1024,
        "M": 1024**2,
        "G": 1024**3,
        "T": 1024**4,
        "P": 1024**5,
    }[suffix]


def deep_merge(base: Mapping[str, Any], override: Mapping[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = json.loads(json.dumps(base))
    for key, value in override.items():
        if isinstance(value, Mapping) and isinstance(result.get(key), Mapping):
            result[key] = deep_merge(result[key], value)
        else:
            result[key] = value
    return result


def load_config(path: Path | None) -> dict[str, Any]:
    if path is None:
        return json.loads(json.dumps(DEFAULT_CONFIG))
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise DemoError(f"config file does not exist: {path}") from exc
    except json.JSONDecodeError as exc:
        raise DemoError(f"invalid JSON config {path}: {exc}") from exc
    if not isinstance(payload, Mapping):
        raise DemoError("config file must contain a JSON object")
    return deep_merge(DEFAULT_CONFIG, payload)


def validate_cluster_name(cluster: str) -> None:
    if len(cluster) > 32 or not CLUSTER_NAME_RE.fullmatch(cluster):
        raise DemoError(
            "cluster must be 1-32 characters, start with a lowercase letter, "
            "and contain only lowercase letters, digits, and hyphens"
        )


def validate_config(config: Mapping[str, Any]) -> None:
    workers = config.get("workers")
    if not isinstance(workers, int) or isinstance(workers, bool) or not 1 <= workers <= 32:
        raise DemoError("workers must be an integer between 1 and 32")
    resources = config.get("resources")
    if not isinstance(resources, Mapping):
        raise DemoError("resources must be an object")
    for role in ("control_plane", "worker"):
        resource = resources.get(role)
        if not isinstance(resource, Mapping):
            raise DemoError(f"resources.{role} must be an object")
        cpus = resource.get("cpus")
        if not isinstance(cpus, int) or isinstance(cpus, bool) or cpus < 1 or cpus > 128:
            raise DemoError(f"resources.{role}.cpus must be between 1 and 128")
        for key in ("memory", "disk"):
            if parse_size(resource.get(key)) < 1024**2:
                raise DemoError(f"resources.{role}.{key} is too small")
    multipass = config.get("multipass")
    if not isinstance(multipass, Mapping):
        raise DemoError("multipass must be an object")
    if not str(multipass.get("image", "")):
        raise DemoError("multipass.image must be set")
    for key in (
        "launch_timeout_seconds",
        "cloud_init_timeout_seconds",
        "ssh_timeout_seconds",
        "build_timeout_seconds",
        "api_timeout_seconds",
    ):
        value = multipass.get(key)
        if not isinstance(value, (int, float)) or value <= 0:
            raise DemoError(f"multipass.{key} must be positive")
    tugboat = config.get("tugboat")
    if not isinstance(tugboat, Mapping):
        raise DemoError("tugboat must be an object")
    if tugboat.get("secure") is not True:
        raise DemoError("the Multipass demo requires secure mode (tugboat.secure=true)")
    if tugboat.get("runtime") != "qemu":
        raise DemoError("the Multipass demo requires the qemu worker runtime")
    if tugboat.get("flannel_mode") != "vxlan":
        raise DemoError("the Multipass demo requires vxlan flannel mode")
    try:
        network = ipaddress.ip_network(str(tugboat.get("cni_subnet")), strict=True)
    except ValueError as exc:
        raise DemoError("tugboat.cni_subnet must be a valid IPv4 network") from exc
    if not isinstance(network, ipaddress.IPv4Network) or not 8 <= network.prefixlen <= 30:
        raise DemoError("tugboat.cni_subnet must be an IPv4 network with a /8-/30 prefix")
    if (
        not isinstance(tugboat.get("build_jobs"), int)
        or isinstance(tugboat["build_jobs"], bool)
        or tugboat["build_jobs"] < 1
    ):
        raise DemoError("tugboat.build_jobs must be a positive integer")
    if not isinstance(tugboat.get("cargo_offline"), bool):
        raise DemoError("tugboat.cargo_offline must be boolean")
    demo = config.get("demo")
    if not isinstance(demo, Mapping) or not str(demo.get("image", "")):
        raise DemoError("demo.image must be set")
    image_digest = str(demo.get("image_digest", ""))
    if image_digest and not re.fullmatch(r"sha256:[0-9a-f]{64}", image_digest):
        raise DemoError("demo.image_digest must be empty or a sha256 digest")
    namespace = str(demo.get("namespace", ""))
    if len(namespace) > 63 or not re.fullmatch(r"[a-z0-9](?:[a-z0-9-]*[a-z0-9])?", namespace):
        raise DemoError("demo.namespace must be a DNS label")
    if namespace in {
        "default",
        "kube-system",
        "kube-public",
        "kube-node-lease",
        "tugboat-system",
    } or namespace.startswith("kube-"):
        raise DemoError("demo.namespace must be a dedicated, non-system namespace")
    probe_config = demo.get("guest_probe", {})
    if not isinstance(probe_config, Mapping):
        raise DemoError("demo.guest_probe must be an object")
    if probe_config.get("enabled"):
        if not isinstance(probe_config.get("ssh_user"), str) or not probe_config["ssh_user"]:
            raise DemoError("an enabled guest probe requires demo.guest_probe.ssh_user")
        identity_file = probe_config.get("ssh_identity_file")
        if not isinstance(identity_file, str) or not Path(identity_file).is_absolute():
            raise DemoError(
                "demo.guest_probe.ssh_identity_file must be an absolute path on the host"
            )
        if not Path(identity_file).is_file():
            raise DemoError(
                "demo.guest_probe.ssh_identity_file must identify a regular file on the host"
            )
        host_public_key = probe_config.get("ssh_host_public_key")
        if not isinstance(host_public_key, str) or not SSH_PUBLIC_KEY_RE.fullmatch(
            host_public_key
        ):
            raise DemoError(
                "demo.guest_probe.ssh_host_public_key must be an OpenSSH public host key"
            )
        if not isinstance(probe_config.get("source_command"), list) or not probe_config["source_command"]:
            raise DemoError("an enabled guest probe requires demo.guest_probe.source_command")
        if not all(isinstance(argument, str) and argument for argument in probe_config["source_command"]):
            raise DemoError("demo.guest_probe.source_command must contain non-empty strings")
        if not str(probe_config.get("peer_address", "")):
            raise DemoError("an enabled guest probe requires demo.guest_probe.peer_address")
        port = probe_config.get("port")
        if not isinstance(port, int) or isinstance(port, bool) or not 1 <= port <= 65535:
            raise DemoError("demo.guest_probe.port must be between 1 and 65535")


def cache_root() -> Path:
    value = os.environ.get("TUGBOAT_MULTIPASS_CACHE_ROOT")
    return Path(value).expanduser().resolve() if value else DEFAULT_CACHE_ROOT


def cluster_dir(cluster: str) -> Path:
    validate_cluster_name(cluster)
    return cache_root() / cluster


def state_paths(directory: Path) -> dict[str, str]:
    return {
        "state": str(directory / "state.json"),
        "inventory": str(directory / "inventory.yml"),
        "vars": str(directory / "vars.json"),
        "ssh_key": str(directory / "ssh" / "id_ed25519"),
        "ssh_public_key": str(directory / "ssh" / "id_ed25519.pub"),
        "known_hosts": str(directory / "ssh" / "known_hosts"),
        "kubeconfig": str(directory / "kubeconfig"),
        "artifacts": str(directory / "artifacts"),
        "build_manifest": str(directory / "build-manifest.json"),
        "demo_report": str(directory / "demo-report.json"),
        "status_report": str(directory / "status-report.json"),
        "cloud_init": str(directory / "cloud-init"),
        "source_archive": str(directory / "source.tar"),
        "logs": str(directory / "logs"),
    }


def ensure_private_directory(path: Path) -> None:
    path.mkdir(parents=True, exist_ok=True)
    os.chmod(path, 0o700)


def _atomic_write(path: Path, content: str, mode: int) -> None:
    parent_created = not path.parent.exists()
    path.parent.mkdir(parents=True, exist_ok=True)
    if parent_created:
        os.chmod(path.parent, 0o700)
    fd, temporary = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    temporary_path = Path(temporary)
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, "w", encoding="utf-8") as handle:
            handle.write(content)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary_path, path)
        os.chmod(path, mode)
    finally:
        if temporary_path.exists():
            temporary_path.unlink()


def write_json_atomic(path: Path, value: Mapping[str, Any], *, mode: int = 0o600) -> None:
    _atomic_write(path, json.dumps(value, indent=2, sort_keys=True) + "\n", mode)


def write_private_text(path: Path, content: str) -> None:
    _atomic_write(path, content, 0o600)


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError as exc:
        raise DemoError(f"state file does not exist: {path}") from exc
    except json.JSONDecodeError as exc:
        raise DemoError(f"invalid JSON state file {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise DemoError(f"JSON object expected in {path}")
    return value


def save_state(directory: Path, state: Mapping[str, Any]) -> None:
    payload = dict(state)
    payload["updated_at"] = now()
    write_json_atomic(Path(payload["paths"]["state"]), payload)


def load_state(directory: Path) -> dict[str, Any]:
    state = read_json(directory / "state.json")
    if state.get("schema") != SCHEMA_VERSION:
        raise DemoError("unsupported Multipass demo state schema")
    if state.get("cluster") != directory.name:
        raise DemoError("state cluster does not match its directory")
    try:
        parsed_uuid = uuid.UUID(str(state.get("cluster_uuid", "")))
    except ValueError as exc:
        raise DemoError("state has no valid cluster UUID")
    if str(parsed_uuid) != str(state["cluster_uuid"]):
        raise DemoError("state has no normalized cluster UUID")
    paths = state.get("paths")
    if not isinstance(paths, Mapping):
        raise DemoError("state contains no path map")
    required_paths = state_paths(directory)
    state_root = directory.resolve()
    for key in required_paths:
        raw_path = paths.get(key)
        if not isinstance(raw_path, str):
            raise DemoError(f"state path is missing: {key}")
        try:
            Path(raw_path).resolve().relative_to(state_root)
        except ValueError as exc:
            raise DemoError(f"state path escapes the cluster directory: {key}") from exc
    state["resources"] = dict(state.get("resources") or {})
    state["resources"].setdefault("source_ref", "")
    nodes = state.get("nodes")
    if not isinstance(nodes, list) or not nodes:
        raise DemoError("state contains no VM nodes")
    for node in nodes:
        if not isinstance(node, dict) or not node.get("name") or node.get("role") not in (
            "control_plane",
            "worker",
        ):
            raise DemoError("state contains an invalid VM node")
    return state


@contextlib.contextmanager
def cluster_lock(directory: Path) -> Iterator[None]:
    ensure_private_directory(directory)
    lock_path = directory / "lock"
    handle = lock_path.open("a+", encoding="utf-8")
    os.chmod(lock_path, 0o600)
    try:
        try:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as exc:
            if exc.errno in (errno.EACCES, errno.EAGAIN):
                raise DemoError(f"cluster is already being operated on: {directory.name}") from exc
            raise
        yield
    finally:
        fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        handle.close()


def make_ssh_keypair(directory: Path, cluster_uuid: str) -> None:
    ssh_dir = directory / "ssh"
    ensure_private_directory(ssh_dir)
    private_key = ssh_dir / "id_ed25519"
    public_key = ssh_dir / "id_ed25519.pub"
    if private_key.exists() and public_key.exists():
        os.chmod(private_key, 0o600)
        os.chmod(public_key, 0o600)
        return
    if private_key.exists() or public_key.exists():
        raise DemoError("SSH keypair is incomplete; remove the cluster state and retry")
    run_command(
        [
            "ssh-keygen",
            "-q",
            "-t",
            "ed25519",
            "-N",
            "",
            "-f",
            str(private_key),
            "-C",
            f"tugboat-multipass:{cluster_uuid}",
        ]
    )
    os.chmod(private_key, 0o600)
    os.chmod(public_key, 0o600)


def render_cloud_init(
    output: Path,
    *,
    ssh_public_key: str,
    cluster_uuid: str,
    node_name: str,
    node_role: str,
) -> None:
    if not TEMPLATE_PATH.exists():
        raise DemoError(f"cloud-init template is missing: {TEMPLATE_PATH}")
    template = TEMPLATE_PATH.read_text(encoding="utf-8")
    replacements = {
        "ssh_public_key": json.dumps(ssh_public_key),
        "cluster_uuid": json.dumps(cluster_uuid),
        "node_name": json.dumps(node_name),
        "node_role": json.dumps(node_role),
    }
    for key, value in replacements.items():
        template = template.replace("{{ " + key + " }}", value)
    if "{{" in template or "}}" in template:
        raise DemoError("cloud-init template contains an unknown placeholder")
    _atomic_write(output, template, 0o600)


def _git_output(argv: Sequence[str]) -> str:
    return run_command(["git", *argv], cwd=REPO_ROOT).stdout


def source_archive(
    output: Path,
    *,
    source_ref: str,
    include_dirty: bool,
) -> tuple[str, str]:
    """Create a deterministic source archive and return its digest and mode."""
    if not include_dirty:
        status = _git_output(["status", "--porcelain", "--untracked-files=all"]).strip()
        if status:
            raise DemoError(
                "the source tree is dirty; commit it or pass --include-dirty explicitly"
            )
        _git_output(["rev-parse", "--verify", "--end-of-options", f"{source_ref}^{{commit}}"])
        temporary = output.with_suffix(".tmp")
        if temporary.exists():
            temporary.unlink()
        run_command(
            ["git", "archive", "--format=tar", "--output", str(temporary), "--", source_ref],
            cwd=REPO_ROOT,
        )
        digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
        os.replace(temporary, output)
        os.chmod(output, 0o600)
        return digest, "git-archive"

    raw = run_command(
        ["git", "ls-files", "-co", "--exclude-standard", "-z"],
        cwd=REPO_ROOT,
    ).stdout
    members = [
        Path(item)
        for item in raw.split("\0")
        if item and not item.startswith((".git/", ".cache/", "target/"))
    ]
    temporary = output.with_suffix(".tmp")
    with tarfile.open(temporary, "w") as archive:
        for relative in sorted(members, key=lambda item: item.as_posix()):
            source = REPO_ROOT / relative
            if not source.exists() and not source.is_symlink():
                continue
            info = archive.gettarinfo(str(source), arcname=relative.as_posix())
            info.uid = 0
            info.gid = 0
            info.uname = ""
            info.gname = ""
            info.mtime = 0
            if info.isreg():
                with source.open("rb") as handle:
                    archive.addfile(info, handle)
            else:
                archive.addfile(info)
    digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
    os.replace(temporary, output)
    os.chmod(output, 0o600)
    return digest, "dirty-snapshot"


def external_binary_manifest(directory: Path) -> tuple[dict[str, Any], str]:
    if not directory.is_dir():
        raise DemoError(f"external prebuilt directory does not exist: {directory}")
    entries: list[dict[str, Any]] = []
    readelf = shutil.which("readelf")
    if not readelf:
        raise DemoError("readelf is required to validate external prebuilt binaries")
    ldconfig = shutil.which("ldconfig")
    library_cache: dict[str, list[Path]] = {}
    if ldconfig:
        cache_result = run_command([ldconfig, "-p"], check=False)
        if cache_result.returncode == 0:
            for line in cache_result.stdout.splitlines():
                match = re.match(r"^\s*(\S+).+=>\s+(\S+)\s*$", line)
                if match:
                    library_cache.setdefault(match.group(1), []).append(Path(match.group(2)))
    for name in REQUIRED_BINARIES:
        path = directory / name
        if not path.is_file() or not os.access(path, os.X_OK):
            raise DemoError(f"external prebuilt binary is missing or not executable: {path}")
        contents = path.read_bytes()
        header = contents[:64]
        if (
            len(header) < 64
            or header[:4] != b"\x7fELF"
            or header[4] != 2
            or header[5] != 1
            or header[6] != 1
            or int.from_bytes(header[16:18], "little") not in (2, 3)
            or int.from_bytes(header[18:20], "little") != 62
            or int.from_bytes(header[20:24], "little") != 1
            or int.from_bytes(header[52:54], "little") < 64
        ):
            raise DemoError(f"external prebuilt binary is not a Linux x86_64 ELF: {path}")
        program_header_offset = int.from_bytes(header[32:40], "little")
        program_header_size = int.from_bytes(header[54:56], "little")
        program_header_count = int.from_bytes(header[56:58], "little")
        entry_point = int.from_bytes(header[24:32], "little")
        program_headers_end = (
            program_header_offset + program_header_size * program_header_count
        )
        if (
            program_header_size < 56
            or program_header_count == 0
            or program_headers_end > len(contents)
        ):
            raise DemoError(f"external prebuilt binary has an invalid ELF layout: {path}")
        entry_point_is_executable = False
        interpreter = ""
        interpreter_seen = False
        for index in range(program_header_count):
            offset = program_header_offset + index * program_header_size
            program_header = contents[offset : offset + program_header_size]
            segment_type = int.from_bytes(program_header[:4], "little")
            segment_offset = int.from_bytes(program_header[8:16], "little")
            segment_virtual_address = int.from_bytes(program_header[16:24], "little")
            segment_file_size = int.from_bytes(program_header[32:40], "little")
            segment_memory_size = int.from_bytes(program_header[40:48], "little")
            segment_alignment = int.from_bytes(program_header[48:56], "little")
            if (
                segment_offset + segment_file_size > len(contents)
                or segment_memory_size < segment_file_size
                or (
                    segment_alignment not in (0, 1)
                    and (
                        segment_alignment & (segment_alignment - 1) != 0
                        or segment_virtual_address % segment_alignment
                        != segment_offset % segment_alignment
                    )
                )
            ):
                raise DemoError(f"external prebuilt binary has an invalid ELF segment: {path}")
            if segment_type == 3:
                raw_interpreter = contents[
                    segment_offset : segment_offset + segment_file_size
                ]
                if (
                    interpreter_seen
                    or len(raw_interpreter) < 2
                    or not raw_interpreter.endswith(b"\0")
                    or b"\0" in raw_interpreter[:-1]
                ):
                    raise DemoError(
                        f"external prebuilt binary has an invalid ELF interpreter: {path}"
                    )
                interpreter_seen = True
                try:
                    interpreter = raw_interpreter[:-1].decode("utf-8")
                except UnicodeDecodeError as exc:
                    raise DemoError(
                        f"external prebuilt binary has an invalid ELF interpreter: {path}"
                    ) from exc
                continue
            if segment_type != 1:
                continue
            segment_flags = int.from_bytes(program_header[4:8], "little")
            if (
                segment_flags & 1
                and segment_file_size > 0
                and segment_virtual_address
                <= entry_point
                < segment_virtual_address + segment_file_size
            ):
                entry_point_is_executable = True
        if entry_point == 0 or not entry_point_is_executable:
            raise DemoError(
                f"external prebuilt binary has no valid executable entry point: {path}"
            )
        if interpreter and (not Path(interpreter).is_absolute() or not Path(interpreter).is_file()):
            raise DemoError(
                f"external prebuilt binary has an unavailable ELF interpreter: "
                f"{path}: {interpreter}"
            )

        readelf_env = os.environ.copy()
        readelf_env["LC_ALL"] = "C"
        readelf_env["LANG"] = "C"
        dynamic = run_command(
            [readelf, "--dynamic", "--wide", str(path)],
            check=False,
            env=readelf_env,
        )
        if dynamic.returncode != 0:
            raise DemoError(f"could not inspect external binary dependencies: {path}")
        dependencies = re.findall(
            r"\(NEEDED\).+Shared library: \[([^\]]+)\]", dynamic.stdout
        )
        runpaths = re.findall(
            r"\((?:RUNPATH|RPATH)\).+Library runpath: \[([^\]]*)\]", dynamic.stdout
        )
        search_directories = [
            path.parent,
            Path("/lib/x86_64-linux-gnu"),
            Path("/usr/lib/x86_64-linux-gnu"),
            Path("/lib64"),
            Path("/usr/lib64"),
            Path("/lib"),
            Path("/usr/lib"),
        ]
        for runpath in runpaths:
            for item in runpath.split(":"):
                expanded = item.replace("${ORIGIN}", str(path.parent)).replace(
                    "$ORIGIN", str(path.parent)
                )
                candidate = Path(expanded)
                if candidate.is_absolute():
                    search_directories.append(candidate)
        unresolved = [
            dependency
            for dependency in dependencies
            if not any((base / dependency).is_file() for base in search_directories)
            and not any(candidate.is_file() for candidate in library_cache.get(dependency, []))
        ]
        if unresolved:
            raise DemoError(
                f"external binary has unresolved shared libraries: {path}: "
                + ", ".join(sorted(unresolved))
            )
        entry: dict[str, Any] = {
            "name": name,
            "sha256": hashlib.sha256(contents).hexdigest(),
            "size": str(path.stat().st_size),
            "shared_libraries": dependencies,
        }
        entries.append(entry)
    identity = {
        "mode": "external-prebuilt",
        "architecture": "x86_64",
        "binaries": entries,
    }
    encoded = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()
    digest = hashlib.sha256(encoded).hexdigest()
    return {**identity, "directory": str(directory)}, digest


def source_artifact_identity(
    config: Mapping[str, Any],
    *,
    source_digest: str,
    source_mode: str,
    source_ref: str,
) -> tuple[dict[str, Any], str]:
    """Build a stable cache/install identity for a source-built artifact."""
    tugboat = config["tugboat"]
    identity = {
        "mode": "source-build",
        "architecture": "x86_64",
        "sourceDigest": source_digest,
        "sourceMode": source_mode,
        "sourceRef": source_ref,
        "toolchain": {"channel": "stable", "profile": "minimal"},
        "build": {
            "profile": "release",
            "locked": True,
            "jobs": tugboat["build_jobs"],
            "cargoOffline": tugboat.get("cargo_offline", False),
        },
        "binaries": list(REQUIRED_BINARIES),
    }
    encoded = json.dumps(identity, sort_keys=True, separators=(",", ":")).encode()
    return identity, hashlib.sha256(encoded).hexdigest()


def host_route_networks() -> list[ipaddress.IPv4Network]:
    result = run_command(["ip", "-4", "route"], check=False)
    if result.returncode != 0:
        details = result.stderr.strip() or "ip route inspection failed"
        raise DemoError(details)
    networks: list[ipaddress.IPv4Network] = []
    for line in result.stdout.splitlines():
        token = line.split()[0] if line.split() else ""
        if token == "default":
            continue
        try:
            network = ipaddress.ip_network(token, strict=False)
        except ValueError:
            continue
        if isinstance(network, ipaddress.IPv4Network):
            networks.append(network)
    return networks


def qemu_kvm_smoke(qemu: str = "qemu-system-x86_64") -> tuple[bool, str]:
    if not Path("/dev/kvm").exists():
        return False, "/dev/kvm is missing"
    if not os.access("/dev/kvm", os.R_OK | os.W_OK):
        return False, "/dev/kvm is not readable and writable"
    argv = [
        qemu,
        "-accel",
        "kvm",
        "-machine",
        "accel=kvm",
        "-nodefaults",
        "-display",
        "none",
        "-monitor",
        "none",
        "-serial",
        "none",
        "-no-reboot",
        "-S",
    ]
    try:
        process = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    except FileNotFoundError:
        return False, f"{qemu} is not installed"
    try:
        time.sleep(0.4)
        if process.poll() is not None:
            stderr = process.stderr.read().strip() if process.stderr else ""
            return False, _redact(stderr) or "QEMU exited before KVM initialization"
        return True, "QEMU started with explicit KVM acceleration"
    finally:
        if process.poll() is None:
            process.send_signal(signal.SIGTERM)
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


def doctor_report(
    mp: Multipass | None = None,
    *,
    cni_subnet: str = "10.244.0.0/16",
) -> dict[str, Any]:
    mp = mp or Multipass()
    checks: dict[str, Any] = {}
    errors: list[str] = []
    for command in (
        "multipass",
        "ansible-playbook",
        "ssh",
        "ssh-keygen",
        "kubectl",
        "ip",
        "git",
    ):
        checks[command] = shutil.which(command) or ""
        if not checks[command]:
            errors.append(f"{command} is not installed")
    qemu_path = shutil.which("qemu-system-x86_64") or ""
    checks["qemu-system-x86_64"] = qemu_path
    checks["host_architecture"] = platform.machine()
    if platform.machine().lower() not in ("x86_64", "amd64"):
        errors.append("the Multipass demo requires an x86_64 host")
    if not qemu_path:
        errors.append("qemu-system-x86_64 is not installed")
    disk_path = cache_root()
    while not disk_path.exists() and disk_path != disk_path.parent:
        disk_path = disk_path.parent
    disk = shutil.disk_usage(disk_path)
    checks["cache_disk_path"] = str(disk_path)
    checks["cache_disk_free_bytes"] = disk.free
    if disk.free < 20 * 1024**3:
        errors.append("the filesystem holding the Multipass cache has less than 20 GiB free")
    checks["kvm"] = {"path": "/dev/kvm", "accessible": os.access("/dev/kvm", os.R_OK | os.W_OK)}
    if not checks["kvm"]["accessible"]:
        errors.append("/dev/kvm is missing or inaccessible")
    if checks["multipass"]:
        try:
            checks["multipass_version"] = mp.version()
            checks["multipass_driver"] = mp.driver()
            checks["multipass_list"] = len(mp.list())
        except DemoError as exc:
            errors.append(str(exc))
        if "multipass_driver" in checks and str(checks["multipass_driver"]).lower() != "qemu":
            errors.append("Multipass local.driver must be qemu")
    if qemu_path and checks["kvm"]["accessible"]:
        ok, message = qemu_kvm_smoke(qemu_path)
        checks["qemu_kvm_smoke"] = message
        if not ok:
            errors.append(message)
    try:
        networks = host_route_networks()
        checks["host_routes"] = [str(network) for network in networks]
        cni_network = ipaddress.ip_network(cni_subnet, strict=True)
        overlaps = [str(network) for network in networks if network.overlaps(cni_network)]
        checks["cni_route_overlaps"] = overlaps
        if overlaps:
            errors.append(f"CNI subnet overlaps host routes: {', '.join(overlaps)}")
    except (DemoError, ValueError) as exc:
        checks["host_routes"] = []
        errors.append(f"cannot inspect host routes: {exc}")
    checks["ok"] = not errors
    checks["errors"] = errors
    return checks


def new_state(cluster: str, config: Mapping[str, Any]) -> dict[str, Any]:
    cluster_uuid = str(uuid.uuid4())
    names = [f"{cluster}-cp-1"] + [f"{cluster}-worker-{index}" for index in range(1, config["workers"] + 1)]
    nodes = [
        {
            "name": names[0],
            "role": "control_plane",
            "index": 1,
            "state": "planned",
            "ip": "",
            "expected_ip": "",
        }
    ]
    nodes.extend(
        {
            "name": name,
            "role": "worker",
            "index": index,
            "state": "planned",
            "ip": "",
            "expected_ip": "",
        }
        for index, name in enumerate(names[1:], start=1)
    )
    directory = cluster_dir(cluster)
    return {
        "schema": SCHEMA_VERSION,
        "cluster": cluster,
        "cluster_uuid": cluster_uuid,
        "created_at": now(),
        "updated_at": now(),
        "stage": "planned",
        "error": "",
        "config": json.loads(json.dumps(config)),
        "nodes": nodes,
        "resources": {
            "source_digest": "",
            "source_mode": "",
            "source_ref": "",
            "artifact_digest": "",
            "external_prebuilt_dir": "",
        },
        "paths": state_paths(directory),
    }


def node_by_name(state: Mapping[str, Any], name: str) -> dict[str, Any]:
    for node in state["nodes"]:
        if node["name"] == name:
            return node
    raise DemoError(f"node is not present in state: {name}")


def expected_names(state: Mapping[str, Any]) -> list[str]:
    return [node["name"] for node in state["nodes"]]


def start_stopped_nodes(mp: Multipass, state: Mapping[str, Any]) -> None:
    listed = {item.get("name"): item for item in mp.list()}
    stopped = [
        name
        for name in expected_names(state)
        if str(listed.get(name, {}).get("state", "")).upper()
        not in ("RUNNING", "STARTING")
    ]
    mp.start(
        stopped,
        timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]),
    )


def stop_running_nodes(mp: Multipass, state: Mapping[str, Any]) -> None:
    listed = {item.get("name"): item for item in mp.list()}
    running = [
        name
        for name in expected_names(state)
        if str(listed.get(name, {}).get("state", "")).upper() == "RUNNING"
    ]
    mp.stop(
        running,
        timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]),
    )


def find_listed(mp: Multipass, name: str) -> dict[str, Any] | None:
    return next((item for item in mp.list() if item.get("name") == name), None)


def verify_owner(
    mp: Multipass,
    state: Mapping[str, Any],
    name: str,
    *,
    start_stopped: bool = True,
    restore_stopped: bool = True,
) -> bool:
    listed = find_listed(mp, name)
    if listed is None:
        raise DemoError(f"expected VM is missing: {name}")
    if str(listed.get("state", "")).upper() == "STARTING":
        wait_for_node(mp, state, {"name": name})
        listed = find_listed(mp, name) or listed
    was_stopped = str(listed.get("state", "")).upper() != "RUNNING"
    started_for_check = False
    if was_stopped and start_stopped:
        mp.start([name], timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]))
        started_for_check = True
    try:
        if started_for_check:
            wait_for_node(mp, state, {"name": name})
        owner = mp.owner(name)
        if owner != state["cluster_uuid"]:
            raise DemoError(
                f"refusing to operate on {name}: owner UUID does not match this cluster"
            )
    except BaseException:
        if started_for_check:
            mp.stop([name], timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]))
        raise
    if started_for_check and restore_stopped:
        mp.stop([name], timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]))
    return started_for_check


def refresh_node(
    mp: Multipass,
    state: dict[str, Any],
    node: dict[str, Any],
    *,
    require_stable_ip: bool = False,
) -> str:
    info = mp.info(node["name"])
    addresses = extract_ipv4_addresses(info)
    cni_subnet = state["config"]["tugboat"]["cni_subnet"]
    address = choose_management_ip(
        addresses,
        cni_subnet,
        lambda candidate: probe_tcp(candidate, 22),
    )
    previous = node.get("ip", "")
    if require_stable_ip and previous and previous != address:
        node["expected_ip"] = previous
        node["state"] = "needs-recreate"
        raise DemoError(
            f"{node['name']} management IP changed from {previous} to {address}; "
            "stop and recreate the cluster to avoid silently reusing data"
        )
    node["ip"] = address
    node["state"] = str(info.get("state", "UNKNOWN")).lower()
    return address


def wait_for_node(mp: Multipass, state: Mapping[str, Any], node: Mapping[str, Any]) -> None:
    timeout = float(state["config"]["multipass"]["launch_timeout_seconds"])
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            info = mp.info(node["name"])
            if str(info.get("state", "")).upper() == "RUNNING":
                mp.exec(
                    node["name"],
                    ["cloud-init", "status", "--wait"],
                    timeout=float(state["config"]["multipass"]["cloud_init_timeout_seconds"]),
                )
                return
            last_error = f"state={info.get('state', 'unknown')}"
        except DemoError as exc:
            last_error = str(exc)
        time.sleep(2)
    raise DemoError(f"{node['name']} did not become ready: {last_error}")


def ssh_argv(state: Mapping[str, Any], node: Mapping[str, Any], command: str) -> list[str]:
    paths = state["paths"]
    return [
        "ssh",
        "-i",
        paths["ssh_key"],
        "-o",
        "BatchMode=yes",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "StrictHostKeyChecking=yes",
        "-o",
        f"UserKnownHostsFile={paths['known_hosts']}",
        "-o",
        "ConnectTimeout=5",
        "-o",
        "LogLevel=ERROR",
        f"ubuntu@{node['ip']}",
        command,
    ]


def wait_for_ssh(mp: Multipass, state: Mapping[str, Any], node: dict[str, Any]) -> None:
    timeout = float(state["config"]["multipass"]["ssh_timeout_seconds"])
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        try:
            result = run_command(
                ssh_argv(state, node, "true"),
                timeout=10,
                check=False,
            )
            if result.returncode == 0:
                node["state"] = "ready"
                return
            last_error = result.stderr.strip()
        except DemoError as exc:
            last_error = str(exc)
        time.sleep(2)
    raise DemoError(f"{node['name']} SSH did not become ready: {_redact(last_error)}")


def write_known_hosts(mp: Multipass, state: Mapping[str, Any]) -> None:
    lines: list[str] = []
    for node in state["nodes"]:
        if not node.get("ip"):
            raise DemoError(f"{node['name']} has no management IP")
        key = mp.host_key(node["name"])
        lines.append(f"{node['ip']} {key}")
        node["host_key"] = key
    write_private_text(Path(state["paths"]["known_hosts"]), "\n".join(lines) + "\n")


def build_inventory(state: Mapping[str, Any]) -> tuple[dict[str, Any], dict[str, Any]]:
    cp = next(node for node in state["nodes"] if node["role"] == "control_plane")
    workers = [node for node in state["nodes"] if node["role"] == "worker"]
    secure = bool(state["config"]["tugboat"]["secure"])
    api_port = 8443 if secure else 8080
    scheme = "https" if secure else "http"
    cp_ip = cp["ip"]
    ssh_args = " ".join(
        [
            "-o StrictHostKeyChecking=yes",
            "-o IdentitiesOnly=yes",
            f"-o UserKnownHostsFile={shlex.quote(state['paths']['known_hosts'])}",
            "-o ConnectTimeout=5",
            "-o LogLevel=ERROR",
        ]
    )
    common_vars: dict[str, Any] = {
        "ansible_user": "ubuntu",
        "ansible_become": True,
        "ansible_python_interpreter": "/usr/bin/python3",
        "ansible_ssh_private_key_file": state["paths"]["ssh_key"],
        "ansible_ssh_common_args": ssh_args,
        "tugboat_build_mode": "prebuilt",
        "tugboat_prebuilt_bin_dir": "/opt/tugboat/bin",
        "tugboat_secure": secure,
        "tugboat_apiserver_listen": f"0.0.0.0:{api_port}",
        "tugboat_apiserver_advertise_url": f"{scheme}://{cp_ip}:{api_port}",
        "tugboat_apiserver_cert_hosts": ["localhost", cp["name"], "tugboat-control-plane"],
        "tugboat_apiserver_cert_ips": ["127.0.0.1", cp_ip],
        "tugboat_pki_dir": "/etc/tugboat/pki",
        "tugboat_force_pki": False,
        "tugboat_etcd_listen": f"{cp_ip}:2379",
        "tugboat_etcd_peer_listen": f"{cp_ip}:2380",
        "tugboat_etcd_node_name": cp["name"],
        "tugboat_etcd_advertise_client_url": f"https://{cp_ip}:2379",
        "tugboat_etcd_initial_advertise_peer_url": f"https://{cp_ip}:2380",
        "tugboat_etcd_initial_cluster": f"{cp['name']}=https://{cp_ip}:2380",
        "tugboat_etcd_initial_cluster_state": "new",
        "tugboat_etcd_endpoints": [f"https://{cp_ip}:2379"],
        "tugboat_etcd_server_cert_hosts": ["localhost", cp["name"]],
        "tugboat_etcd_server_cert_ips": ["127.0.0.1", cp_ip],
        "tugboat_etcd_peer_cert_hosts": ["localhost", cp["name"]],
        "tugboat_etcd_peer_cert_ips": ["127.0.0.1", cp_ip],
        "tugboat_etcd_data_dir": "/var/lib/tugboat-etcd",
        "tugboat_etcd_pki_dir": "/etc/tugboat/pki/etcd",
        "tugboat_flannel_mode": "vxlan",
        "tugboat_cni_subnet": state["config"]["tugboat"]["cni_subnet"],
        "tugboat_flannel_etcd_endpoints": f"https://{cp_ip}:2379",
        "tugboat_flannel_etcd_ca": "/etc/tugboat/pki/etcd/ca.crt",
        "tugboat_flannel_etcd_cert": "/etc/tugboat/pki/etcd/client.crt",
        "tugboat_flannel_etcd_key": "/etc/tugboat/pki/etcd/client.key",
        "tugboat_worker_runtime": "qemu",
        "tugboat_csi_hostpath_enabled": False,
        "tugboat_control_plane_install_checksum": state["resources"].get("artifact_digest", ""),
        "tugboat_worker_install_checksum": state["resources"].get("artifact_digest", ""),
    }
    hostvars: dict[str, Any] = {}
    hostvars[cp["name"]] = {"ansible_host": cp_ip, "tugboat_node_name": cp["name"]}
    for worker in workers:
        hostvars[worker["name"]] = {
            "ansible_host": worker["ip"],
            "tugboat_node_name": worker["name"],
        }
    inventory = {
        "all": {
            "vars": common_vars,
            "hosts": hostvars,
            "children": {
                "tugboat_control_plane": {"hosts": {cp["name"]: {}}},
                "tugboat_workers": {
                    "hosts": {worker["name"]: {} for worker in workers}
                },
                "tugboat_csi_hostpath": {"hosts": {}},
            },
        }
    }
    vars_payload = {
        "tugboat_multipass_cluster": state["cluster"],
        "tugboat_multipass_cluster_uuid": state["cluster_uuid"],
        "tugboat_multipass_source_archive": state["paths"]["source_archive"],
        "tugboat_multipass_source_digest": state["resources"].get("source_digest", ""),
        "tugboat_multipass_source_mode": state["resources"].get("source_mode", ""),
        "tugboat_multipass_source_ref": state["resources"].get("source_ref", ""),
        "tugboat_multipass_source_enabled": state["resources"].get("external_prebuilt_dir", "") == "",
        "tugboat_multipass_external_prebuilt_bin_dir": state["resources"].get(
            "external_prebuilt_dir", ""
        ),
        "tugboat_multipass_artifact_digest": state["resources"].get("artifact_digest", ""),
        "tugboat_multipass_required_binaries": list(REQUIRED_BINARIES),
        "tugboat_build_jobs": state["config"]["tugboat"]["build_jobs"],
        "tugboat_multipass_cargo_offline": state["config"]["tugboat"].get("cargo_offline", False),
        "tugboat_multipass_local_manifest_path": state["paths"]["build_manifest"],
        "tugboat_multipass_local_artifacts_dir": state["paths"]["artifacts"],
        "tugboat_multipass_local_kubeconfig_path": state["paths"]["kubeconfig"],
        "tugboat_multipass_local_demo_report_path": state["paths"]["demo_report"],
        "tugboat_multipass_local_status_report_path": state["paths"]["status_report"],
        "tugboat_multipass_local_logs_dir": state["paths"]["logs"],
        "tugboat_multipass_expected_workers": len(workers),
        "tugboat_multipass_expected_control_plane_ip": cp_ip,
        "tugboat_multipass_api_url": f"{scheme}://{cp_ip}:{api_port}",
        "tugboat_multipass_api_port": api_port,
        "tugboat_multipass_api_ca_path": "/etc/tugboat/pki/ca.crt",
        "tugboat_demo_namespace": state["config"]["demo"]["namespace"],
        "tugboat_demo_image": state["config"]["demo"]["image"],
        "tugboat_demo_image_digest": state["config"]["demo"].get("image_digest", ""),
        "tugboat_demo_token_audience": state["config"]["demo"].get(
            "token_audience", "https://localhost:8443"
        ),
        "tugboat_demo_guest_probe": state["config"]["demo"].get("guest_probe", {}),
    }
    return inventory, vars_payload


def write_inventory(state: Mapping[str, Any]) -> None:
    inventory, vars_payload = build_inventory(state)
    write_json_atomic(Path(state["paths"]["inventory"]), inventory)
    write_json_atomic(Path(state["paths"]["vars"]), vars_payload)


def run_playbook(
    state: Mapping[str, Any],
    playbook: str,
    *,
    extra_vars: Mapping[str, Any] | None = None,
    timeout: float | None = None,
) -> None:
    write_inventory(state)
    variables = read_json(Path(state["paths"]["vars"]))
    if extra_vars:
        variables.update(extra_vars)
    write_json_atomic(Path(state["paths"]["vars"]), variables)
    env = os.environ.copy()
    env["ANSIBLE_CONFIG"] = str(ANSIBLE_ROOT / "ansible.cfg")
    env["ANSIBLE_HOST_KEY_CHECKING"] = "true"
    env["ANSIBLE_RETRY_FILES_ENABLED"] = "false"
    env["ANSIBLE_LOCAL_TEMP"] = str(Path(state["paths"]["logs"]) / "ansible-tmp")
    env["ANSIBLE_REMOTE_TEMP"] = "/tmp/.ansible/tmp"
    ensure_private_directory(Path(state["paths"]["logs"]))
    ensure_private_directory(Path(state["paths"]["artifacts"]))
    result = run_command(
        [
            "ansible-playbook",
            "-i",
            state["paths"]["inventory"],
            "--extra-vars",
            f"@{state['paths']['vars']}",
            str((ANSIBLE_ROOT if playbook == "site.yml" else MULTIPASS_ANSIBLE_ROOT) / playbook),
        ],
        cwd=ANSIBLE_ROOT,
        env=env,
        timeout=timeout,
        check=False,
    )
    _atomic_write(
        Path(state["paths"]["logs"]) / f"{Path(playbook).stem}.stdout.log",
        result.stdout,
        0o600,
    )
    _atomic_write(
        Path(state["paths"]["logs"]) / f"{Path(playbook).stem}.stderr.log",
        result.stderr,
        0o600,
    )
    if result.returncode != 0:
        raise DemoError(
            f"Ansible playbook failed: {playbook}\n"
            f"stdout: {Path(state['paths']['logs']) / (Path(playbook).stem + '.stdout.log')}\n"
            f"stderr: {Path(state['paths']['logs']) / (Path(playbook).stem + '.stderr.log')}"
        )


def ensure_source_and_artifacts(
    state: dict[str, Any],
    *,
    source_ref: str,
    include_dirty: bool,
    external_prebuilt_dir: Path | None,
) -> None:
    resources = state["resources"]
    ensure_private_directory(Path(state["paths"]["artifacts"]))
    if external_prebuilt_dir:
        manifest, digest = external_binary_manifest(external_prebuilt_dir)
        write_json_atomic(Path(state["paths"]["build_manifest"]), manifest)
        resources["external_prebuilt_dir"] = str(external_prebuilt_dir.resolve())
        resources["source_digest"] = ""
        resources["source_mode"] = "external-prebuilt"
        resources["source_ref"] = ""
        resources["artifact_digest"] = digest
        return
    digest, mode = source_archive(
        Path(state["paths"]["source_archive"]),
        source_ref=source_ref,
        include_dirty=include_dirty,
    )
    identity, artifact_digest = source_artifact_identity(
        state["config"],
        source_digest=digest,
        source_mode=mode,
        source_ref=source_ref,
    )
    resources["source_digest"] = digest
    resources["source_mode"] = mode
    resources["source_ref"] = source_ref
    resources["external_prebuilt_dir"] = ""
    resources["artifact_digest"] = artifact_digest
    write_json_atomic(
        Path(state["paths"]["build_manifest"]),
        {
            **identity,
            "artifactDigest": artifact_digest,
        },
    )


def ensure_vms(mp: Multipass, state: dict[str, Any]) -> None:
    config = state["config"]
    listed = {item["name"]: item for item in mp.list()}
    public_key = Path(state["paths"]["ssh_public_key"]).read_text(encoding="utf-8").strip()
    cloud_dir = Path(state["paths"]["cloud_init"])
    ensure_private_directory(cloud_dir)

    # Check every existing name before launching any missing VM. This prevents
    # a collision on a later worker from leaving a partially created cluster.
    for node in state["nodes"]:
        if node["name"] in listed:
            verify_owner(
                mp,
                state,
                node["name"],
                start_stopped=True,
                restore_stopped=False,
            )

    for node in state["nodes"]:
        existing = listed.get(node["name"])
        if existing:
            continue
        cloud_init = cloud_dir / f"{node['name']}.yaml"
        render_cloud_init(
            cloud_init,
            ssh_public_key=public_key,
            cluster_uuid=state["cluster_uuid"],
            node_name=node["name"],
            node_role=node["role"],
        )
        resource_key = "control_plane" if node["role"] == "control_plane" else "worker"
        resource = config["resources"][resource_key]
        node["state"] = "launching"
        save_state(cluster_dir(state["cluster"]), state)
        mp.launch(
            image=config["multipass"]["image"],
            name=node["name"],
            cpus=resource["cpus"],
            memory=resource["memory"],
            disk=resource["disk"],
            cloud_init=cloud_init,
            timeout=float(config["multipass"]["launch_timeout_seconds"]),
        )


def prepare_nodes(mp: Multipass, state: dict[str, Any], *, require_stable_ip: bool = False) -> None:
    for node in state["nodes"]:
        verify_owner(mp, state, node["name"], start_stopped=False)
        wait_for_node(mp, state, node)
        refresh_node(mp, state, node, require_stable_ip=require_stable_ip)
        save_state(cluster_dir(state["cluster"]), state)
    write_known_hosts(mp, state)
    for node in state["nodes"]:
        wait_for_ssh(mp, state, node)
        save_state(cluster_dir(state["cluster"]), state)


def ensure_doctor(cni_subnet: str) -> None:
    report = doctor_report(cni_subnet=cni_subnet)
    if not report["ok"]:
        raise DemoError("host preflight failed:\n- " + "\n- ".join(report["errors"]))


def reconcile_existing_config(
    state: dict[str, Any],
    config: dict[str, Any],
    args: argparse.Namespace,
) -> dict[str, Any]:
    saved = deep_merge(DEFAULT_CONFIG, state["config"])
    state["config"] = saved

    # A retry without --config should use the persisted configuration. Apply
    # only explicit CLI overrides on top so a custom CNI or timeout does not
    # unexpectedly turn back into the defaults on the next invocation.
    if not args.config:
        config = json.loads(json.dumps(saved))
        overrides = {
            "workers": ("workers",),
            "cpus": ("resources", "control_plane", "cpus"),
            "memory": ("resources", "control_plane", "memory"),
            "disk": ("resources", "control_plane", "disk"),
            "worker_cpus": ("resources", "worker", "cpus"),
            "worker_memory": ("resources", "worker", "memory"),
            "worker_disk": ("resources", "worker", "disk"),
            "image": ("multipass", "image"),
            "cni_subnet": ("tugboat", "cni_subnet"),
        }
        for argument, path in overrides.items():
            value = getattr(args, argument)
            if value is None:
                continue
            target: Any = config
            for component in path[:-1]:
                target = target[component]
            target[path[-1]] = value

    if state["stage"] != "planned":
        for key in ("multipass", "workers", "resources", "tugboat"):
            if config.get(key) != saved.get(key):
                raise DemoError(
                    f"existing cluster configuration differs in {key}; use its saved config "
                    "or destroy it first"
                )
    if config["workers"] != saved["workers"]:
        raise DemoError("worker count cannot be changed on an existing cluster")
    return config


def load_or_create_state(cluster: str, config: dict[str, Any], args: argparse.Namespace) -> tuple[Path, dict[str, Any]]:
    directory = cluster_dir(cluster)
    state_file = directory / "state.json"
    if state_file.exists():
        state = load_state(directory)
        config = reconcile_existing_config(state, config, args)
        validate_config(config)
        state["config"] = config
        return directory, state
    ensure_private_directory(directory)
    state = new_state(cluster, config)
    save_state(directory, state)
    return directory, state


def cluster_needs_recreate(state: Mapping[str, Any]) -> bool:
    return state.get("stage") == "needs-recreate" or any(
        node.get("state") == "needs-recreate" for node in state.get("nodes", [])
    )


def record_operation_failure(
    directory: Path,
    state: dict[str, Any],
    exc: BaseException,
) -> None:
    state["stage"] = "needs-recreate" if cluster_needs_recreate(state) else "failed"
    state["error"] = str(exc)
    save_state(directory, state)


def run_up(args: argparse.Namespace) -> int:
    validate_cluster_name(args.cluster)
    config = config_from_args(args)
    validate_config(config)
    directory = cluster_dir(args.cluster)
    ensure_private_directory(directory)
    with cluster_lock(directory):
        directory, state = load_or_create_state(args.cluster, config, args)
        config = state["config"]
        try:
            if cluster_needs_recreate(state):
                raise DemoError(
                    "this cluster has an IP change recorded; destroy and recreate it explicitly"
                )
            ensure_doctor(config["tugboat"]["cni_subnet"])
            make_ssh_keypair(directory, state["cluster_uuid"])
            artifacts_missing = (
                not state["resources"].get("artifact_digest")
                or not Path(state["paths"]["build_manifest"]).exists()
            )
            source_requested = (
                args.include_dirty
                or args.source_ref != "HEAD"
                or bool(args.external_prebuilt_dir)
            )
            if artifacts_missing or source_requested:
                ensure_source_and_artifacts(
                    state,
                    source_ref=args.source_ref,
                    include_dirty=args.include_dirty,
                    external_prebuilt_dir=Path(args.external_prebuilt_dir).expanduser()
                    if args.external_prebuilt_dir
                    else None,
                )
            save_state(directory, state)
            state["stage"] = "launching"
            state["error"] = ""
            save_state(directory, state)
            mp = Multipass()
            ensure_vms(mp, state)
            start_stopped_nodes(mp, state)
            state["stage"] = "waiting"
            save_state(directory, state)
            prepare_nodes(mp, state, require_stable_ip=True)
            save_state(directory, state)
            if not args.skip_prepare:
                run_playbook(
                    state,
                    "prepare.yml",
                    timeout=float(config["multipass"]["build_timeout_seconds"]),
                )
            run_playbook(
                state,
                "site.yml",
                timeout=float(config["multipass"]["api_timeout_seconds"]),
            )
            if not args.skip_verify:
                run_playbook(
                    state,
                    "verify.yml",
                    timeout=float(config["multipass"]["api_timeout_seconds"]),
                )
            state["stage"] = "verified"
            save_state(directory, state)
            print(f"cluster {args.cluster} is ready")
            return 0
        except (Exception, KeyboardInterrupt) as exc:
            record_operation_failure(directory, state, exc)
            try:
                run_playbook(state, "collect.yml", timeout=120)
            except Exception:
                pass
            raise


def run_provision(args: argparse.Namespace) -> int:
    validate_cluster_name(args.cluster)
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        config = config_from_args(args)
        validate_config(config)
        config = reconcile_existing_config(state, config, args)
        validate_config(config)
        state["config"] = config
        try:
            if cluster_needs_recreate(state):
                raise DemoError(
                    "this cluster has an IP change recorded; destroy and recreate it explicitly"
                )
            mp = Multipass()
            for node in state["nodes"]:
                verify_owner(
                    mp,
                    state,
                    node["name"],
                    start_stopped=True,
                    restore_stopped=False,
                )
            start_stopped_nodes(mp, state)
            prepare_nodes(mp, state, require_stable_ip=True)
            if (
                args.update_source
                or args.external_prebuilt_dir
                or not state["resources"].get("artifact_digest")
                or not Path(state["paths"]["build_manifest"]).exists()
            ):
                ensure_source_and_artifacts(
                    state,
                    source_ref=args.source_ref,
                    include_dirty=args.include_dirty,
                    external_prebuilt_dir=Path(args.external_prebuilt_dir).expanduser()
                    if args.external_prebuilt_dir
                    else None,
                )
                save_state(directory, state)
            run_playbook(
                state,
                "prepare.yml",
                timeout=float(config["multipass"]["build_timeout_seconds"]),
            )
            run_playbook(
                state,
                "site.yml",
                timeout=float(config["multipass"]["api_timeout_seconds"]),
            )
            if not args.skip_verify:
                run_playbook(state, "verify.yml", timeout=float(config["multipass"]["api_timeout_seconds"]))
            state["stage"] = "verified"
            state["error"] = ""
            save_state(directory, state)
            print(f"cluster {args.cluster} provisioned")
            return 0
        except (Exception, KeyboardInterrupt) as exc:
            record_operation_failure(directory, state, exc)
            raise


def read_guest_json(mp: Multipass, name: str, path: str) -> dict[str, Any]:
    output = mp.exec(name, ["sudo", "cat", path], timeout=30)
    try:
        value = json.loads(output)
    except json.JSONDecodeError as exc:
        raise DemoError(f"{name}: invalid JSON in {path}") from exc
    if not isinstance(value, dict):
        raise DemoError(f"{name}: expected an object in {path}")
    return value


def refuse_stale_cluster(state: Mapping[str, Any]) -> None:
    if cluster_needs_recreate(state):
        raise DemoError(
            "this cluster has an IP change recorded; destroy and recreate it explicitly"
        )


def run_demo(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        validate_config(state["config"])
        refuse_stale_cluster(state)
        workers = [node for node in state["nodes"] if node["role"] == "worker"]
        if len(workers) < 2:
            raise DemoError("the demo requires at least two workers")
        mp = Multipass()
        if args.cleanup:
            for node in state["nodes"]:
                verify_owner(mp, state, node["name"], start_stopped=False)
            run_playbook(
                state,
                "demo.yml",
                extra_vars={
                    "tugboat_demo_apply": False,
                    "tugboat_demo_issue_token": False,
                    "tugboat_demo_cleanup": True,
                },
                timeout=float(state["config"]["multipass"]["api_timeout_seconds"]),
            )
            state["stage"] = "verified"
            state["error"] = ""
            state["resources"].pop("demo", None)
            for report_key in ("demo_report", "status_report"):
                with contextlib.suppress(FileNotFoundError):
                    Path(state["paths"][report_key]).unlink()
            save_state(directory, state)
            print(f"demo resources for {args.cluster} cleaned up")
            return 0
        if not state["config"]["demo"]["guest_probe"].get("enabled"):
            raise DemoError(
                "the demo requires demo.guest_probe.enabled=true so guest-to-guest "
                "service connectivity is verified"
            )
        for node in state["nodes"]:
            verify_owner(mp, state, node["name"], start_stopped=False)
        run_playbook(
            state,
            "demo.yml",
            extra_vars={"tugboat_demo_apply": True, "tugboat_demo_issue_token": True},
            timeout=float(state["config"]["multipass"]["api_timeout_seconds"]),
        )
        report = read_guest_json(
            mp,
            next(node for node in state["nodes"] if node["role"] == "control_plane")["name"],
            "/var/lib/tugboat/demo/report.json",
        )
        write_json_atomic(Path(state["paths"]["demo_report"]), report)
        state["resources"]["demo"] = report
        state["stage"] = "demo-verified"
        save_state(directory, state)
        print(json.dumps(report, indent=2, sort_keys=True))
        return 0


def run_kubeconfig(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        refuse_stale_cluster(state)
        mp = Multipass()
        cp = next(node for node in state["nodes"] if node["role"] == "control_plane")
        verify_owner(mp, state, cp["name"], start_stopped=False)
        run_playbook(
            state,
            "demo.yml",
            extra_vars={
                "tugboat_demo_apply": False,
                "tugboat_demo_issue_token": True,
                "tugboat_demo_write_kubeconfig": True,
            },
            timeout=float(state["config"]["multipass"]["api_timeout_seconds"]),
        )
        output = Path(args.output).expanduser() if args.output else Path(state["paths"]["kubeconfig"])
        content = mp.exec(cp["name"], ["sudo", "cat", "/var/lib/tugboat/demo/kubeconfig"], timeout=30)
        write_private_text(output, content)
        if output != Path(state["paths"]["kubeconfig"]):
            write_private_text(Path(state["paths"]["kubeconfig"]), content)
        run_command(
            ["kubectl", "--kubeconfig", str(output), "get", "nodes", "-o", "name"],
            timeout=30,
        )
        state["stage"] = "kubeconfig-ready"
        save_state(directory, state)
        print(output)
        return 0


def run_status(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        mp = Multipass()
        listed = {item.get("name"): item for item in mp.list()}
        output = {
            "cluster": state["cluster"],
            "stage": state["stage"],
            "error": state.get("error", ""),
            "expected_workers": state["config"]["workers"],
            "nodes": [],
        }
        owners_match = True
        for node in state["nodes"]:
            item = listed.get(node["name"], {})
            owner = ""
            if item and str(item.get("state", "")).upper() == "RUNNING":
                try:
                    owner = mp.owner(node["name"])
                except DemoError:
                    owner = "unavailable"
                if owner != state["cluster_uuid"]:
                    owners_match = False
            output["nodes"].append(
                {
                    "name": node["name"],
                    "role": node["role"],
                    "state": item.get("state", "MISSING"),
                    "ip": node.get("ip", ""),
                    "owner": owner,
                }
            )
        all_running = all(
            str(listed.get(node["name"], {}).get("state", "")).upper() == "RUNNING"
            and node.get("ip")
            for node in state["nodes"]
        )
        if (
            all_running
            and owners_match
            and state["stage"] != "planned"
            and not cluster_needs_recreate(state)
        ):
            try:
                run_playbook(
                    state,
                    "verify.yml",
                    extra_vars={
                        "tugboat_multipass_check_demo": state["stage"]
                        in ("demo-verified", "kubeconfig-ready")
                    },
                    timeout=float(state["config"]["multipass"]["api_timeout_seconds"]),
                )
                output["verification"] = "passed"
            except DemoError as exc:
                output["verification"] = "failed"
                output["verification_error"] = str(exc)
        else:
            output["verification"] = "not-run" if owners_match else "blocked-owner"
        status_report = Path(state["paths"]["status_report"])
        if status_report.exists():
            output["demo_ships"] = read_json(status_report)
        if args.json:
            print(json.dumps(output, indent=2, sort_keys=True))
        else:
            print(f"cluster: {output['cluster']}")
            print(f"stage:   {output['stage']}")
            print(f"workers: {output['expected_workers']}")
            print(f"verify:  {output['verification']}")
            for node in output["nodes"]:
                print(f"{node['role']:14} {node['name']:28} {node['state']:10} {node['ip']}")
            if output["error"]:
                print(f"error:   {output['error']}", file=sys.stderr)
            if output.get("verification_error"):
                print(f"verify error: {output['verification_error']}", file=sys.stderr)
        return 0


def run_start(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        mp = Multipass()
        try:
            if cluster_needs_recreate(state):
                raise DemoError(
                    "this cluster has an IP change recorded; destroy and recreate it explicitly"
                )
            for node in state["nodes"]:
                verify_owner(
                    mp,
                    state,
                    node["name"],
                    start_stopped=True,
                    restore_stopped=False,
                )
            start_stopped_nodes(mp, state)
            prepare_nodes(mp, state, require_stable_ip=True)
            run_playbook(state, "verify.yml", timeout=float(state["config"]["multipass"]["api_timeout_seconds"]))
            state["stage"] = "verified"
            state["error"] = ""
            save_state(directory, state)
            print(f"cluster {args.cluster} started")
            return 0
        except (Exception, KeyboardInterrupt) as exc:
            record_operation_failure(directory, state, exc)
            try:
                stop_running_nodes(mp, state)
            except Exception:
                pass
            raise


def run_stop(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        mp = Multipass()
        for node in state["nodes"]:
            verify_owner(mp, state, node["name"], start_stopped=True)
        stop_running_nodes(mp, state)
        if not cluster_needs_recreate(state):
            state["stage"] = "stopped"
        save_state(directory, state)
        print(f"cluster {args.cluster} stopped")
        return 0


def run_collect(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        refuse_stale_cluster(state)
        mp = Multipass()
        listed = {item.get("name"): item for item in mp.list()}
        originally_stopped = [
            name
            for name in expected_names(state)
            if name in listed
            and str(listed[name].get("state", "")).upper() != "RUNNING"
        ]
        started_for_collect: list[str] = []
        try:
            for node in state["nodes"]:
                started = verify_owner(
                    mp,
                    state,
                    node["name"],
                    start_stopped=True,
                    restore_stopped=False,
                )
                if started and node["name"] in originally_stopped:
                    started_for_collect.append(node["name"])
            start_stopped_nodes(mp, state)
            run_playbook(state, "collect.yml", timeout=120)
        finally:
            if started_for_collect:
                mp.stop(
                    started_for_collect,
                    timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]),
                )
        print(f"diagnostics collected in {state['paths']['logs']}")
        return 0


def destroy_local_secrets(directory: Path, state: Mapping[str, Any]) -> None:
    names = (
        "state",
        "inventory",
        "vars",
        "ssh_key",
        "ssh_public_key",
        "known_hosts",
        "kubeconfig",
        "build_manifest",
        "demo_report",
        "status_report",
        "source_archive",
    )
    for key in names:
        path = Path(state["paths"][key])
        if path.exists():
            path.unlink()
    cloud_dir = Path(state["paths"]["cloud_init"])
    if cloud_dir.exists():
        shutil.rmtree(cloud_dir)
    artifacts_dir = Path(state["paths"]["artifacts"])
    if artifacts_dir.exists():
        shutil.rmtree(artifacts_dir)
    ssh_dir = directory / "ssh"
    if ssh_dir.exists():
        with contextlib.suppress(OSError):
            ssh_dir.rmdir()


def run_destroy(args: argparse.Namespace) -> int:
    directory = cluster_dir(args.cluster)
    with cluster_lock(directory):
        state = load_state(directory)
        mp = Multipass()
        names = expected_names(state)
        for node in state["nodes"]:
            if find_listed(mp, node["name"]):
                verify_owner(mp, state, node["name"], start_stopped=True)
        print("destroy targets: " + ", ".join(names))
        if not args.yes:
            try:
                answer = input(f"Type the cluster name '{args.cluster}' to destroy it: ")
            except EOFError as exc:
                raise DemoError("destroy requires --yes when no interactive input is available") from exc
            if answer.strip() != args.cluster:
                raise DemoError("destroy cancelled")
        try:
            present_names = [name for name in names if find_listed(mp, name)]
            mp.delete(
                present_names,
                timeout=float(state["config"]["multipass"]["launch_timeout_seconds"]),
            )
        except Exception as exc:
            state["stage"] = "delete-incomplete"
            state["error"] = str(exc)
            save_state(directory, state)
            raise
        residual = [name for name in names if find_listed(mp, name)]
        if residual:
            state["stage"] = "delete-incomplete"
            state["error"] = "VMs remain after delete: " + ", ".join(residual)
            save_state(directory, state)
            raise DemoError(state["error"])
        logs = Path(state["paths"]["logs"])
        destroy_local_secrets(directory, state)
        with contextlib.suppress(OSError):
            (directory / "lock").unlink()
        print(f"cluster {args.cluster} destroyed; diagnostics remain in {logs}")
        return 0


def config_from_args(args: argparse.Namespace) -> dict[str, Any]:
    config = load_config(Path(args.config).expanduser() if args.config else None)
    if args.workers is not None:
        config["workers"] = args.workers
    if args.cpus is not None:
        config["resources"]["control_plane"]["cpus"] = args.cpus
    if args.memory is not None:
        config["resources"]["control_plane"]["memory"] = args.memory
    if args.disk is not None:
        config["resources"]["control_plane"]["disk"] = args.disk
    if args.worker_cpus is not None:
        config["resources"]["worker"]["cpus"] = args.worker_cpus
    if args.worker_memory is not None:
        config["resources"]["worker"]["memory"] = args.worker_memory
    if args.worker_disk is not None:
        config["resources"]["worker"]["disk"] = args.worker_disk
    if args.image is not None:
        config["multipass"]["image"] = args.image
    if args.cni_subnet is not None:
        config["tugboat"]["cni_subnet"] = args.cni_subnet
    return config


def add_cluster_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("cluster_positional", nargs="?")
    parser.add_argument("--cluster", dest="cluster_option", default="")
    parser.add_argument("--config", default="")
    parser.add_argument("--workers", type=int, default=None)
    parser.add_argument("--cpus", type=int, default=None)
    parser.add_argument("--memory", default=None)
    parser.add_argument("--disk", default=None)
    parser.add_argument("--worker-cpus", type=int, default=None)
    parser.add_argument("--worker-memory", default=None)
    parser.add_argument("--worker-disk", default=None)
    parser.add_argument("--image", default=None)
    parser.add_argument("--cni-subnet", default=None)


def make_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    doctor = subparsers.add_parser("doctor", help="check host prerequisites")
    doctor.add_argument("--json", action="store_true")

    up = subparsers.add_parser("up", help="create and provision a cluster")
    add_cluster_arguments(up)
    up.add_argument("--source-ref", default="HEAD")
    up.add_argument("--include-dirty", action="store_true")
    up.add_argument("--external-prebuilt-dir", default="")
    up.add_argument("--skip-prepare", action="store_true")
    up.add_argument("--skip-verify", action="store_true")

    provision = subparsers.add_parser("provision", help="reapply Ansible to an existing cluster")
    add_cluster_arguments(provision)
    provision.add_argument("--update-source", action="store_true")
    provision.add_argument("--source-ref", default="HEAD")
    provision.add_argument("--include-dirty", action="store_true")
    provision.add_argument("--external-prebuilt-dir", default="")
    provision.add_argument("--skip-verify", action="store_true")

    for command in ("demo", "kubeconfig", "status", "stop", "start", "collect", "destroy"):
        command_parser = subparsers.add_parser(command)
        add_cluster_arguments(command_parser)
        if command == "kubeconfig":
            command_parser.add_argument("--output", default="")
        elif command == "demo":
            command_parser.add_argument("--cleanup", action="store_true")
        elif command == "status":
            command_parser.add_argument("--json", action="store_true")
        elif command == "destroy":
            command_parser.add_argument("--yes", action="store_true")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = make_parser()
    args = parser.parse_args(argv)
    if args.command != "doctor":
        args.cluster = args.cluster_option or args.cluster_positional
        if not args.cluster:
            parser.error("a cluster name is required as an argument or with --cluster")
        if args.cluster_option and args.cluster_positional:
            parser.error("provide the cluster name either positionally or with --cluster")
    try:
        if args.command == "doctor":
            report = doctor_report()
            print(json.dumps(report, indent=2, sort_keys=True) if args.json else "\n".join(
                [f"{key}: {value}" for key, value in report.items() if key != "errors"]
                + ([f"error: {error}" for error in report["errors"]] if report["errors"] else [])
            ))
            return 0 if report["ok"] else 1
        return {
            "up": run_up,
            "provision": run_provision,
            "demo": run_demo,
            "kubeconfig": run_kubeconfig,
            "status": run_status,
            "stop": run_stop,
            "start": run_start,
            "collect": run_collect,
            "destroy": run_destroy,
        }[args.command](args)
    except DemoError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("interrupted", file=sys.stderr)
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
