[Unit]
Description=etcd key-value store (tugboat)
Documentation=https://etcd.io
After=network.target

[Service]
Type=notify
User=tugboat-etcd
ExecStart=/usr/local/bin/etcd \
    --listen-client-urls http://${ETCD_LISTEN} \
    --advertise-client-urls http://${ETCD_LISTEN} \
    --data-dir ${DATA_DIR}
Restart=on-failure
RestartSec=5s
StateDirectory=tugboat-etcd
StateDirectoryMode=0700
NoNewPrivileges=true
ProtectSystem=full

[Install]
WantedBy=multi-user.target
