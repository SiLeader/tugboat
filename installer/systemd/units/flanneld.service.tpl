[Unit]
Description=flanneld overlay network agent
After=network-online.target
Wants=network-online.target
Before=tugboat-agent.service

[Service]
User=root
ExecStartPre=/usr/bin/install -d -m 0755 /run/flannel /var/lib/cni/flannel
ExecStartPre=-/usr/sbin/modprobe br_netfilter
ExecStartPre=/usr/bin/rm -f /run/flannel/subnet.env
ExecStart=/opt/cni/bin/flanneld \
    --etcd-endpoints=${FLANNEL_ETCD_ENDPOINTS} \
    --etcd-prefix=${FLANNEL_ETCD_PREFIX} \
    --subnet-file=/run/flannel/subnet.env \
    ${FLANNEL_ETCD_TLS_ARGS} \
    --iptables-forward-rules=false \
    --ip-masq=false
Restart=on-failure
RestartSec=5s

# flanneld configures host networking devices and routes.
AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_DAC_READ_SEARCH
CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_DAC_READ_SEARCH
PrivateNetwork=false
ProtectSystem=full

[Install]
WantedBy=multi-user.target
