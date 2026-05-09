[cloud_hypervisor]
executable = "/usr/local/bin/cloud-hypervisor"
disk_image_location = "/var/lib/tugboat-agent/images"

# TODO: replace this with the real vmlinux path for this host.
[cloud_hypervisor.boot]
kernel = "/var/lib/tugboat-agent/vmlinux"
