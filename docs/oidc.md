# OIDC Integration

Tugboat supports OpenID Connect (OIDC) as an authentication method. This allows you to integrate Tugboat with external identity providers (IdPs) like Dex, Keycloak, Okta, or Google.

## Configuration

OIDC is configured in the `tugboat-apiserver` configuration file. You can define multiple OIDC providers.

```toml
[[authentication.oidc]]
issuer_url = "https://dex.example.com"
client_id = "tugboat"
username_claim = "email"
username_prefix = "oidc:"
groups_claim = "groups"
groups_prefix = "oidc:"
# optional: restrict access to users with specific claims
# required_claims = { hd = "example.com" }
ca_file = "/etc/tugboat/oidc/dex-ca.crt"
```

### Key Parameters

- **issuer_url**: The base URL of the OIDC provider. Tugboat will fetch `/.well-known/openid-configuration` from this URL.
- **client_id**: The client ID issued by the IdP. The `aud` claim in the ID token must match this value.
- **username_claim**: The claim to use as the Tugboat username (e.g., `email`, `sub`).
- **username_prefix**: A prefix added to the username to prevent collisions with local users (e.g., `oidc:`).
- **groups_claim**: The claim to use for group membership.
- **groups_prefix**: A prefix added to each group name.

## Usage in RBAC

Once OIDC is configured, you can use the prefixed usernames and groups in `RoleBinding` and `ClusterRoleBinding` subjects.

```yaml
apiVersion: authorization/v1
kind: RoleBinding
metadata:
  name: oidc-view-binding
  namespace: default
subjects:
  - kind: Group
    name: oidc:developers
    apiGroup: authorization/v1
roleRef:
  kind: ClusterRole
  name: view
  apiGroup: authorization/v1
```

## Emergency Access

If the OIDC provider is down, you can still access the cluster using:
1. Service Account tokens (if you have one saved).
2. Client certificates (mTLS).
3. Always-allow mode (not recommended for production).

It is recommended to keep at least one local `cluster-admin` identity (e.g., a Service Account or a client certificate) for emergency use.
