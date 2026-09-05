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

#### Implicit `namespace` and `ca.crt` files in any projected volume

Unlike Kubernetes — where `namespace` comes from a `downwardAPI` projection and `ca.crt` from a `configMap` projection that the user writes explicitly — Tugboat injects both files automatically into **every** projected volume (whether or not it contains a `serviceAccountToken` projection), as long as the user did not already write files at those paths.

This means a `projected` volume composed only of `configMap` and `secret` projections will still produce a `namespace` file (and `ca.crt` when the agent is configured with an apiserver CA path) alongside the user-declared files. If you need to suppress them, declare your own projection that writes to those paths.

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

RS256 signing keys may use PKCS#8 or PKCS#1 PEM/DER encoding. They must have a
2048-4096-bit modulus and a public exponent of at least 65537. Additional RSA
verification keys may have a 2048-8192-bit modulus.
