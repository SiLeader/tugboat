[http]
listen = "${APISERVER_LISTEN}"

[etcd]
endpoints = ["${ETCD_ENDPOINT}"]

[authorization]
mode = "AlwaysAllow"

[authentication]
anonymous_enabled = true
