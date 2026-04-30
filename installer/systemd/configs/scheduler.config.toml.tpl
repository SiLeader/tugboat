[apiserver]
url = "${APISERVER_URL}"

[apiserver.auth]
type = "anonymous"

[scheduler]
name = "default-scheduler"
lease_duration_seconds = 15
renew_interval_seconds = 10
scheduling_interval_seconds = 1

[scheduler.plugins]
filter = ["NetworkFit", "TaintToleration", "RuntimeClassFit", "ResourceFit"]
score = ["TaintToleration", "LeastAllocated"]
