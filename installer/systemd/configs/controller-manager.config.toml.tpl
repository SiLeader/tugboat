[apiserver]
url = "${APISERVER_URL}"
${APISERVER_CLIENT_TLS_CONFIG}

${CONTROLLER_MANAGER_APISERVER_AUTH_CONFIG}

[csi]
requeue_interval_seconds = 30
socket_connect_timeout_seconds = 5
rpc_timeout_seconds = 30

[network]
requeue_interval_seconds = 30

[csi.provisioners."hostpath.csi.k8s.io"]
socket_path = "/var/run/csi/csi.sock"
