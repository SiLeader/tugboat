[Unit]
Description=Tugboat API Server
After=etcd.service
Requires=etcd.service

[Service]
User=tugboat-apiserver
ExecStart=/usr/local/bin/tugboat-apiserver --config /etc/tugboat/apiserver/config.toml
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/log/tugboat/apiserver
StateDirectory=tugboat-apiserver

[Install]
WantedBy=multi-user.target
