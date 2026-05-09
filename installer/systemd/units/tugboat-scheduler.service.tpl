[Unit]
Description=Tugboat Scheduler
After=tugboat-apiserver.service
Wants=tugboat-apiserver.service

[Service]
User=tugboat-scheduler
ExecStart=/usr/local/bin/tugboat-scheduler --config /etc/tugboat/scheduler/config.toml
Restart=on-failure
RestartSec=5s
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/var/log/tugboat/scheduler
StateDirectory=tugboat-scheduler

[Install]
WantedBy=multi-user.target
