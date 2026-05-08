[Unit]
Description=Hostpath CSI Provisioner
After=network-online.target
Wants=network-online.target

[Service]
User=tugboat-csi-hostpath
Group=tugboat-csi-hostpath
UMask=0027
ExecStartPre=+/usr/bin/install -d -m 0755 -o tugboat-csi-hostpath -g tugboat-csi-hostpath /var/run/csi
ExecStartPre=+/usr/bin/install -d -m 0755 -o tugboat-csi-hostpath -g tugboat-csi-hostpath ${HOSTPATH_DATA_DIR}
ExecStartPre=/usr/bin/rm -f /var/run/csi/csi.sock
ExecStart=/usr/local/bin/hostpathplugin \
    --endpoint=unix:///var/run/csi/csi.sock \
    --nodeid=${HOSTPATH_NODE_ID} \
    --statedir=${HOSTPATH_DATA_DIR} \
    --logtostderr \
    --v=5
Restart=on-failure
RestartSec=5s

# Filesystem publishing can require mount operations when workloads use the driver.
AmbientCapabilities=CAP_SYS_ADMIN
CapabilityBoundingSet=CAP_SYS_ADMIN
PrivateMounts=false
ProtectSystem=strict
ReadWritePaths=/var/run/csi ${HOSTPATH_DATA_DIR}

[Install]
WantedBy=multi-user.target
