# Service Account Tokens

Service Accounts provide identities for processes that run in Ships or for system components. Tugboat supports two types of tokens for Service Accounts: Opaque Bearer Tokens and Signed JWT Tokens.

## Opaque Bearer Tokens (Legacy)

Opaque tokens are 64-character random strings stored in a Secret of type `service-account-token`. These tokens are long-lived and do not have an inherent expiry or audience.

## Signed JWT Tokens

Signed JWT tokens are modern, short-lived tokens that include claims like issuer (`iss`), subject (`sub`), audience (`aud`), and expiration (`exp`).

### TokenRequest API

Tokens can be requested dynamically via the `TokenRequest` API:

```bash
# Requesting a token via curl
curl -X POST -H "Authorization: Bearer $ADMIN_TOKEN" \
     -d '{"spec": {"audiences": ["https://apiserver.tugboat.cloud"], "expirationSeconds": 3600}}' \
     https://apiserver.tugboat.cloud/v1/namespaces/default/serviceaccounts/my-sa/token
```

### Token Projection into Ships

Tugboat can automatically mount a Service Account token into a Ship. This is called "token projection".

When `automountServiceAccountToken: true` is set in the Ship spec (which is the default), the following files are mounted at `/var/run/secrets/tugboat.cloud/serviceaccount/`:

- `token`: The signed JWT token.
- `ca.crt`: The CA certificate for the apiserver.
- `namespace`: The namespace of the Ship.

#### Rotation

The Tugboat agent automatically rotates the projected token when it reaches 80% of its lifetime (or when it has 10 minutes remaining, whichever is shorter).

## Configuration

Service Account token signing is configured in the apiserver:

```toml
[authentication.service_account]
issuer = "https://apiserver.tugboat.cloud"
signing_key_file = "/etc/tugboat/pki/tugboat-apiserver-sa-signing.key"
signing_algorithm = "RS256"
default_token_ttl_seconds = 3600
max_token_ttl_seconds = 86400
```
