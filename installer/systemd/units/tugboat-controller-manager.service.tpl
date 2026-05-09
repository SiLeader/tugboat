[Unit]
Description=Tugboat Controller Manager
After=tugboat-apiserver.service
Wants=tugboat-apiserver.service

[Service]
User=tugboat-controller-manager
ExecStart=/usr/local/bin/tugboat-controller-manager \
    --config /etc/tugboat/controller-manager/config.toml
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/log/tugboat/controller-manager
StateDirectory=tugboat-controller-manager

[Install]
WantedBy=multi-user.target
