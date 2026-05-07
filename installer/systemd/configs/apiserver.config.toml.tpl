[http]
listen = "${APISERVER_LISTEN}"
${APISERVER_TLS_CONFIG}

[etcd]
endpoints = ["${ETCD_ENDPOINT}"]

[authorization]
mode = "AlwaysAllow"

[authentication]
anonymous_enabled = true
