[Unit]
Description=Tugboat Agent
After=network-online.target
Wants=network-online.target
${AGENT_FLANNEL_UNIT_DEPENDENCIES}

[Service]
User=root
ExecStart=/usr/local/bin/tugboat-agent --config /etc/tugboat/agent/config.toml
Restart=on-failure
RestartSec=5s

# Capabilities required for VM operations, network namespaces, and mounts.
AmbientCapabilities=CAP_NET_ADMIN CAP_SYS_ADMIN CAP_NET_RAW
CapabilityBoundingSet=CAP_NET_ADMIN CAP_SYS_ADMIN CAP_NET_RAW

# Keep host network and mount namespace access available to the agent.
PrivateNetwork=false
PrivateMounts=false
Delegate=yes

# Writable state used by the agent and child runtime processes.
StateDirectory=tugboat-agent
RuntimeDirectory=tugboat-agent
RuntimeDirectoryPreserve=yes

# The agent needs broad host write access for runtime and CNI operations.
ProtectSystem=no

[Install]
WantedBy=multi-user.target
