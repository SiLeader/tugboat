# Ship

A `Ship` is the smallest deployable unit in Tugboat, representing a single Virtual Machine.

## Manifest Example

```yaml
apiVersion: core/v1
kind: Ship
metadata:
  name: example-ship
  namespace: default
spec:
  shipClassName: standard
  serviceAccountName: my-service-account
  automountServiceAccountToken: true
```

## Service Account Token Projection

By default, Tugboat automatically mounts a Service Account token into the Ship. This token can be used by applications inside the VM to authenticate with the Tugboat API server.

- **Mount Path**: `/var/run/secrets/tugboat.cloud/serviceaccount/`
- **Files**:
    - `token`: Signed JWT token
    - `ca.crt`: API server CA certificate
    - `namespace`: The Ship's namespace

The token is automatically rotated by the node agent.
