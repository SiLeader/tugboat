[Unit]
Description=etcd key-value store (tugboat)
Documentation=https://etcd.io
After=network.target

[Service]
Type=notify
User=tugboat-etcd
ExecStart=/usr/local/bin/etcd \
    --name ${ETCD_NODE_NAME} \
    --listen-client-urls https://${ETCD_LISTEN} \
    --advertise-client-urls ${ETCD_ADVERTISE_CLIENT_URL} \
    --listen-peer-urls ${ETCD_LISTEN_PEER_URL} \
    --initial-advertise-peer-urls ${ETCD_INITIAL_ADVERTISE_PEER_URL} \
    --initial-cluster ${ETCD_INITIAL_CLUSTER} \
    --initial-cluster-state ${ETCD_INITIAL_CLUSTER_STATE} \
    --data-dir ${DATA_DIR} \
    --cert-file ${ETCD_PKI_DIR}/server.crt \
    --key-file ${ETCD_PKI_DIR}/server.key \
    --trusted-ca-file ${ETCD_PKI_DIR}/ca.crt \
    --client-cert-auth \
    --peer-cert-file ${ETCD_PKI_DIR}/peer.crt \
    --peer-key-file ${ETCD_PKI_DIR}/peer.key \
    --peer-trusted-ca-file ${ETCD_PKI_DIR}/ca.crt \
    --peer-client-cert-auth
Restart=on-failure
RestartSec=5s
StateDirectory=tugboat-etcd
StateDirectoryMode=0700
NoNewPrivileges=true
ProtectSystem=full

[Install]
WantedBy=multi-user.target
