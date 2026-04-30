[node]
name = "${NODE_NAME}"
network_probe_interval_seconds = 30

[runtime]
executable = "/usr/local/bin/${RUNTIME_BINARY}"
args = ["--config", "/etc/tugboat/runtime/${RUNTIME_CONFIG_FILE}"]

[apiserver]
url = "${APISERVER_URL}"

[apiserver.auth]
type = "anonymous"

[image]
cache_dir = "/var/lib/tugboat-agent/images"

[cni]

[csi]
publish_dir = "/var/lib/tugboat-agent/csi"
socket_connect_timeout_seconds = 5
rpc_timeout_seconds = 30
