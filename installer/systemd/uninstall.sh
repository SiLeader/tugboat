#!/usr/bin/env bash
# shellcheck source=installer/systemd/lib.sh
set -Eeuo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPT_DIR}/lib.sh"

CONTROL_PLANE=0
WORKER=0
PURGE=0

CONTROL_PLANE_UNITS=(
    hostpath-provisioner.service
    tugboat-controller-manager.service
    tugboat-scheduler.service
    tugboat-apiserver.service
    etcd.service
)

CONTROL_PLANE_BINARIES=(
    hostpathplugin
    tugboat-apiserver
    tugboat-scheduler
    tugboat-controller-manager
)

WORKER_BINARIES=(
    hostpathplugin
    tugboat-agent
    tugboat-qemu-runtime
    tugboat-cloud-hypervisor-runtime
)

CNI_PLUGIN_BINARIES=(
    bandwidth
    bridge
    dhcp
    dummy
    firewall
    flannel
    flanneld
    host-device
    host-local
    ipvlan
    loopback
    macvlan
    portmap
    ptp
    sbr
    static
    tap
    tuning
    vlan
    vrf
)

usage() {
    cat <<'USAGE'
Usage: uninstall.sh (--control-plane | --worker | --control-plane --worker) [--purge]

Options:
  --control-plane  Remove control-plane systemd units, binaries, and configs.
  --worker         Remove worker systemd unit, binaries, CNI tmpfiles config, and configs.
  --purge          Also remove Tugboat data directories and installed CNI plugin binaries.
USAGE
}

parse_args() {
    if [[ "$#" -eq 0 ]]; then
        usage >&2
        return 2
    fi

    while [[ "$#" -gt 0 ]]; do
        case "$1" in
            --control-plane)
                CONTROL_PLANE=1
                shift
                ;;
            --worker)
                WORKER=1
                shift
                ;;
            --purge)
                PURGE=1
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

    if [[ "${CONTROL_PLANE}" -ne 1 && "${WORKER}" -ne 1 ]]; then
        log_error "At least one of --control-plane or --worker is required."
        usage >&2
        return 2
    fi
}

remove_binaries() {
    local binary

    for binary in "$@"; do
        rm -f -- "/usr/local/bin/${binary}"
    done
}

remove_cni_plugins() {
    local binary

    for binary in "${CNI_PLUGIN_BINARIES[@]}"; do
        rm -f -- "${CNI_BIN_DIR}/${binary}"
    done
}

remove_control_plane() {
    local unit

    log_info "Removing control-plane services."
    for unit in "${CONTROL_PLANE_UNITS[@]}"; do
        disable_unit "${unit}"
    done

    rm -f -- \
        "${SYSTEMD_UNIT_DIR}/hostpath-provisioner.service" \
        "${SYSTEMD_UNIT_DIR}/etcd.service" \
        "${SYSTEMD_UNIT_DIR}/tugboat-apiserver.service" \
        "${SYSTEMD_UNIT_DIR}/tugboat-scheduler.service" \
        "${SYSTEMD_UNIT_DIR}/tugboat-controller-manager.service"
    systemctl daemon-reload

    remove_binaries "${CONTROL_PLANE_BINARIES[@]}"
    rm -rf -- \
        /etc/tugboat/apiserver \
        /etc/tugboat/scheduler \
        /etc/tugboat/controller-manager \
        /etc/tugboat/pki
    rmdir --ignore-fail-on-non-empty /etc/tugboat 2>/dev/null || true

    if [[ "${PURGE}" -eq 1 ]]; then
        rm -rf -- \
            /var/lib/tugboat-apiserver \
            /var/lib/tugboat-scheduler \
            /var/lib/tugboat-controller-manager \
            /var/lib/tugboat-csi-hostpath \
            /var/run/csi \
            /var/lib/tugboat-etcd
    fi
}

remove_worker() {
    log_info "Removing worker services."
    disable_unit tugboat-agent.service
    disable_unit flanneld.service
    disable_unit hostpath-provisioner.service

    rm -f -- \
        "${SYSTEMD_UNIT_DIR}/tugboat-agent.service" \
        "${SYSTEMD_UNIT_DIR}/flanneld.service" \
        "${SYSTEMD_UNIT_DIR}/hostpath-provisioner.service"
    systemctl daemon-reload

    remove_binaries "${WORKER_BINARIES[@]}"
    rm -f -- "${TMPFILES_DIR}/tugboat-flannel.conf"
    rm -rf -- /etc/tugboat/agent /etc/tugboat/runtime
    rm -f -- /etc/tugboat/pki/ca.crt
    rmdir --ignore-fail-on-non-empty /etc/tugboat/pki 2>/dev/null || true
    rmdir --ignore-fail-on-non-empty /etc/tugboat 2>/dev/null || true

    if [[ "${PURGE}" -eq 1 ]]; then
        rm -rf -- \
            /var/lib/tugboat-agent \
            /var/lib/tugboat-csi-hostpath \
            /var/lib/cni/flannel \
            /var/run/csi \
            /run/flannel
        remove_cni_plugins
    fi
}

main() {
    parse_args "$@"
    require_root

    if [[ "${CONTROL_PLANE}" -eq 1 ]]; then
        remove_control_plane
    fi

    if [[ "${WORKER}" -eq 1 ]]; then
        remove_worker
    fi

    log_info "Uninstall complete."
}

main "$@"
