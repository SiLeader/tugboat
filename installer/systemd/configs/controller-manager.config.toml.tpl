[apiserver]
url = "${APISERVER_URL}"

[apiserver.auth]
type = "anonymous"

[csi]
requeue_interval_seconds = 30
socket_connect_timeout_seconds = 5
rpc_timeout_seconds = 30

[network]
requeue_interval_seconds = 30
