[apiserver]
url = "${APISERVER_URL}"
${APISERVER_CLIENT_TLS_CONFIG}

${SCHEDULER_APISERVER_AUTH_CONFIG}

[scheduler]
name = "default-scheduler"
lease_duration_seconds = 15
renew_interval_seconds = 10
scheduling_interval_seconds = 1

[scheduler.plugins]
filter = ["NetworkFit", "TaintToleration", "RuntimeClassFit", "ResourceFit"]
score = ["TaintToleration", "LeastAllocated"]
