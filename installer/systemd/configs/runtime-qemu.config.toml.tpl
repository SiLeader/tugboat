[qemu]
disk_image_location = "/var/lib/tugboat-agent/images"

[qemu.executables]
qemu = "/usr/bin/qemu-system-x86_64"
qemu_img = "/usr/bin/qemu-img"

[qemu.kvm]
enabled = true

[qemu.uefi]
code_file = "/usr/share/OVMF/OVMF_CODE_4M.fd"
vars_file = "/usr/share/OVMF/OVMF_VARS_4M.fd"
