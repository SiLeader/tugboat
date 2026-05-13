# Audit Logging

Audit logging provides a structured record of all API requests made to the `tugboat-apiserver`. This is essential for security auditing, compliance, and troubleshooting.

## Configuration

Audit logging is configured in the `tugboat-apiserver` configuration file.

```toml
[audit]
enabled = true
log_path = "/var/log/tugboat/audit.log"
max_size_mb = 100
max_backups = 5
max_age_days = 30

# Policy Rules
[[audit.rules]]
level = "RequestResponse"
verbs = ["create", "update", "patch", "delete"]

[[audit.rules]]
level = "Metadata"
```

### Audit Levels

- **None**: Do not log events that match this rule.
- **Metadata**: Log request metadata (user, timestamp, resource, verb, response code) but not the request or response bodies.
- **Request**: Log metadata and the request body.
- **RequestResponse**: Log metadata, request body, and response body.

## Log Format

Audit logs are written as JSON lines. Each line represents a single `AuditEvent`.

Example:

```json
{
  "kind": "Event",
  "apiVersion": "audit.tugboat.cloud/v1",
  "level": "Metadata",
  "auditID": "550e8400-e29b-41d4-a716-446655440000",
  "stage": "ResponseComplete",
  "requestURI": "/v1/namespaces/default/ships",
  "verb": "list",
  "user": {
    "username": "oidc:alice@example.com",
    "groups": ["system:authenticated", "oidc:developers"]
  },
  "sourceIPs": ["192.168.1.10"],
  "userAgent": "tugboat-cli/0.1.0",
  "objectRef": {
    "resource": "ships",
    "namespace": "default",
    "apiGroup": "core",
    "apiVersion": "v1"
  },
  "responseStatus": {"code": 200},
  "requestReceivedTimestamp": "2026-05-11T10:00:00Z",
  "stageTimestamp": "2026-05-11T10:00:00.050Z",
  "annotations": {
    "authorization.tugboat.cloud/decision": "allow",
    "authorization.tugboat.cloud/reason": "RBAC: allowed by RoleBinding/view-binding"
  }
}
```

## Policy Best Practices

1. **Exclude high-volume noise**: Set `level = "None"` for resources like `leases` that are updated frequently.
2. **Protect sensitive data**: Tugboat automatically forces `Metadata` level for `Secrets` and `ServiceAccount` tokens to avoid leaking sensitive information in the logs.
3. **Use Metadata for read operations**: Use `Metadata` for `get`, `list`, and `watch` to save disk space.
4. **Use RequestResponse for mutations**: Use higher levels for `create`, `update`, and `patch` to capture exactly what changed.

## Log Rotation

Tugboat automatically rotates logs daily. You can also configure size-based rotation using `max_size_mb`. Old logs are kept for `max_age_days` or until `max_backups` is reached.
