[http]
listen = "${APISERVER_LISTEN}"
${APISERVER_TLS_CONFIG}

[etcd]
endpoints = ["${ETCD_ENDPOINT}"]

[authorization]
mode = "${APISERVER_AUTHORIZATION_MODE}"

[authentication]
anonymous_enabled = ${APISERVER_ANONYMOUS_ENABLED}
