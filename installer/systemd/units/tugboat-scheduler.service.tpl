[Unit]
Description=Tugboat Scheduler
After=tugboat-apiserver.service
Wants=tugboat-apiserver.service

[Service]
User=tugboat
ExecStart=/usr/local/bin/tugboat-scheduler --config /etc/tugboat/scheduler/config.toml
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
ProtectSystem=strict
StateDirectory=tugboat-scheduler

[Install]
WantedBy=multi-user.target
