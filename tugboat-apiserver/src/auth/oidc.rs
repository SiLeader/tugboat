// Copyright 2025- SiLeader (Cerussite).
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! External OIDC identity provider verification.
//!
//! Validates ID tokens issued by configured OIDC providers (Dex, Keycloak,
//! Auth0, cloud IAM …) using each provider's published JWKS. The provider is
//! selected by matching the `iss` claim against the configured providers.
//! When no provider matches, verification fails closed (no fallback to the
//! opaque-token path) so a JWT with a foreign issuer can never be silently
//! interpreted as something else.

use crate::config::OidcProviderConfig;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openssl::bn::BigNum;
use openssl::hash::MessageDigest;
use openssl::pkey::{Id, PKey, Public};
use openssl::rsa::Rsa;
use openssl::sign::Verifier;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::warn;

const JWT_LEEWAY_SECONDS: u64 = 60;
const DISCOVERY_TIMEOUT_SECONDS: u64 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VerifiedOidcIdentity {
    pub(crate) issuer: String,
    pub(crate) username: String,
    pub(crate) groups: Vec<String>,
}

pub(crate) struct OidcAuthenticator {
    providers: Vec<OidcProvider>,
}

impl OidcAuthenticator {
    pub(crate) fn from_config(configs: &[OidcProviderConfig]) -> Result<Option<Self>, String> {
        if configs.is_empty() {
            return Ok(None);
        }
        let mut providers = Vec::with_capacity(configs.len());
        for config in configs {
            providers.push(OidcProvider::from_config(config)?);
        }
        if let Some(duplicate) = duplicate_issuer(&providers) {
            return Err(format!(
                "duplicate OIDC issuer configured: {duplicate}; each [[authentication.oidc]] block must use a distinct issuer_url"
            ));
        }
        Ok(Some(Self { providers }))
    }

    /// Returns the provider that should verify `token`, by matching the
    /// (unverified) `iss` claim against the configured providers. `None` means
    /// the token is not OIDC-shaped or its issuer is not registered.
    pub(crate) fn provider_for_token(&self, token: &str) -> ProviderMatch<'_> {
        let Some(iss) = peek_issuer(token) else {
            return ProviderMatch::NotJwt;
        };
        match self.providers.iter().find(|p| p.issuer_url == iss) {
            Some(provider) => ProviderMatch::Matched(provider),
            None => ProviderMatch::UnknownIssuer(iss),
        }
    }

    pub(crate) async fn verify(
        &self,
        provider: &OidcProvider,
        token: &str,
    ) -> Result<VerifiedOidcIdentity, OidcVerifyError> {
        provider.verify_token(token).await
    }
}

pub(crate) enum ProviderMatch<'a> {
    Matched(&'a OidcProvider),
    /// Token has an `iss` claim but it does not match any configured provider.
    UnknownIssuer(String),
    /// Token is not a 3-segment JWT or its claims could not be decoded.
    NotJwt,
}

pub(crate) struct OidcProvider {
    issuer_url: String,
    client_id: String,
    username_claim: String,
    username_prefix: String,
    groups_claim: String,
    groups_prefix: String,
    required_claims: HashMap<String, String>,
    http_client: reqwest::Client,
    cache: RwLock<JwksCacheState>,
    refresh_interval: Duration,
    min_refresh_interval: Duration,
}

#[derive(Default)]
struct JwksCacheState {
    jwks_uri: Option<String>,
    keys: HashMap<String, JwksEntry>,
    last_refreshed: Option<Instant>,
}

struct JwksEntry {
    algorithm: SupportedAlgorithm,
    key: PKey<Public>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SupportedAlgorithm {
    RS256,
    EdDSA,
}

impl SupportedAlgorithm {
    fn parse(value: &str) -> Result<Self, OidcVerifyError> {
        match value {
            "RS256" => Ok(Self::RS256),
            "EdDSA" => Ok(Self::EdDSA),
            other => Err(OidcVerifyError::UnsupportedAlgorithm(other.to_string())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum OidcVerifyError {
    #[error("OIDC token is not a well-formed JWT: {0}")]
    Malformed(String),
    #[error("OIDC discovery failed for {issuer}: {source}")]
    Discovery {
        issuer: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("OIDC JWKS fetch failed for {issuer}: {source}")]
    Jwks {
        issuer: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("OIDC signing algorithm {0} is not supported")]
    UnsupportedAlgorithm(String),
    #[error("OIDC token kid {0} is not present in the issuer JWKS")]
    UnknownKid(String),
    #[error("OIDC token signature is invalid")]
    InvalidSignature,
    #[error("OIDC token claim is invalid: {0}")]
    Claim(String),
}

impl OidcProvider {
    fn from_config(config: &OidcProviderConfig) -> Result<Self, String> {
        if config.issuer_url.is_empty() {
            return Err("issuer_url must not be empty".to_string());
        }
        if config.client_id.is_empty() {
            return Err("client_id must not be empty".to_string());
        }
        if config.username_prefix.is_empty() {
            warn!(
                issuer = %config.issuer_url,
                "OIDC provider configured without username_prefix; OIDC users may collide with local identities"
            );
        }
        let http_client = build_http_client(config)?;
        Ok(Self {
            issuer_url: config.issuer_url.trim_end_matches('/').to_string(),
            client_id: config.client_id.clone(),
            username_claim: config.username_claim.clone(),
            username_prefix: config.username_prefix.clone(),
            groups_claim: config.groups_claim.clone(),
            groups_prefix: config.groups_prefix.clone(),
            required_claims: config.required_claims.clone(),
            http_client,
            cache: RwLock::new(JwksCacheState::default()),
            refresh_interval: Duration::from_secs(config.jwks_refresh_seconds),
            min_refresh_interval: Duration::from_secs(config.jwks_min_refresh_seconds),
        })
    }

    async fn verify_token(&self, token: &str) -> Result<VerifiedOidcIdentity, OidcVerifyError> {
        let parts = split_jwt(token)?;
        let header: JwtHeader = decode_segment(parts.header)?;
        let key = self.lookup_key(&header.kid, &header.alg).await?;
        if header.alg != algorithm_str(key.algorithm) {
            return Err(OidcVerifyError::Claim(
                "JWT header alg does not match JWKS key alg".to_string(),
            ));
        }
        let signing_input = format!("{}.{}", parts.header, parts.claims);
        let signature = URL_SAFE_NO_PAD
            .decode(parts.signature)
            .map_err(|err| OidcVerifyError::Malformed(format!("signature not base64url: {err}")))?;
        verify_signature(&key, signing_input.as_bytes(), &signature)?;

        let claims: Value = decode_segment(parts.claims)?;
        validate_claims(self, &claims)?;
        let identity = build_identity(self, &claims)?;
        Ok(identity)
    }

    async fn lookup_key(&self, kid: &str, alg: &str) -> Result<JwksEntry, OidcVerifyError> {
        // Fast path: cache hit + fresh.
        {
            let cache = self.cache.read().await;
            if !is_stale(&cache, self.refresh_interval)
                && let Some(entry) = cache.keys.get(kid)
            {
                return Ok(entry.clone_for_use());
            }
        }
        // Slow path: refresh, then retry lookup once.
        self.refresh_jwks().await?;
        let cache = self.cache.read().await;
        match cache.keys.get(kid) {
            Some(entry) => Ok(entry.clone_for_use()),
            None => {
                // Useful when diagnosing IdP key rotation issues; alg comes from
                // the unverified header but is safe to log.
                warn!(
                    issuer = %self.issuer_url,
                    kid = %kid,
                    alg = %alg,
                    "OIDC token references unknown kid after JWKS refresh"
                );
                Err(OidcVerifyError::UnknownKid(kid.to_string()))
            }
        }
    }

    async fn refresh_jwks(&self) -> Result<(), OidcVerifyError> {
        // Rate-limit: skip if a recent refresh already happened.
        {
            let cache = self.cache.read().await;
            if let Some(last) = cache.last_refreshed
                && last.elapsed() < self.min_refresh_interval
            {
                return Ok(());
            }
        }
        let jwks_uri = self.resolve_jwks_uri().await?;
        let document = self
            .http_client
            .get(&jwks_uri)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|err| OidcVerifyError::Jwks {
                issuer: self.issuer_url.clone(),
                source: Box::new(err),
            })?
            .json::<JwksDocument>()
            .await
            .map_err(|err| OidcVerifyError::Jwks {
                issuer: self.issuer_url.clone(),
                source: Box::new(err),
            })?;
        let keys = parse_jwks(document)?;

        let mut cache = self.cache.write().await;
        cache.jwks_uri = Some(jwks_uri);
        cache.keys = keys;
        cache.last_refreshed = Some(Instant::now());
        Ok(())
    }

    async fn resolve_jwks_uri(&self) -> Result<String, OidcVerifyError> {
        // Discovery cached on first refresh; subsequent refreshes reuse it.
        {
            let cache = self.cache.read().await;
            if let Some(uri) = &cache.jwks_uri {
                return Ok(uri.clone());
            }
        }
        let discovery_url = format!("{}/.well-known/openid-configuration", self.issuer_url);
        let discovery: DiscoveryDocument = self
            .http_client
            .get(&discovery_url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|err| OidcVerifyError::Discovery {
                issuer: self.issuer_url.clone(),
                source: Box::new(err),
            })?
            .json()
            .await
            .map_err(|err| OidcVerifyError::Discovery {
                issuer: self.issuer_url.clone(),
                source: Box::new(err),
            })?;
        if discovery.issuer.trim_end_matches('/') != self.issuer_url {
            return Err(OidcVerifyError::Discovery {
                issuer: self.issuer_url.clone(),
                source: format!(
                    "discovery document issuer '{}' does not match configured issuer_url '{}'",
                    discovery.issuer, self.issuer_url
                )
                .into(),
            });
        }
        Ok(discovery.jwks_uri)
    }

    #[cfg(test)]
    async fn seed_keys_for_test(
        &self,
        jwks_uri: impl Into<String>,
        keys: HashMap<String, JwksEntry>,
    ) {
        let mut cache = self.cache.write().await;
        cache.jwks_uri = Some(jwks_uri.into());
        cache.keys = keys;
        cache.last_refreshed = Some(Instant::now());
    }
}

impl JwksEntry {
    fn clone_for_use(&self) -> Self {
        Self {
            algorithm: self.algorithm,
            // PKey is cheap to clone; it wraps an Arc internally.
            key: self.key.clone(),
        }
    }
}

fn duplicate_issuer(providers: &[OidcProvider]) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    for provider in providers {
        if !seen.insert(provider.issuer_url.clone()) {
            return Some(provider.issuer_url.clone());
        }
    }
    None
}

fn is_stale(cache: &JwksCacheState, ttl: Duration) -> bool {
    match cache.last_refreshed {
        None => true,
        Some(at) => at.elapsed() >= ttl,
    }
}

fn algorithm_str(algorithm: SupportedAlgorithm) -> &'static str {
    match algorithm {
        SupportedAlgorithm::RS256 => "RS256",
        SupportedAlgorithm::EdDSA => "EdDSA",
    }
}

fn build_http_client(config: &OidcProviderConfig) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(DISCOVERY_TIMEOUT_SECONDS))
        .https_only(false);
    if let Some(ca_path) = &config.ca_file {
        let bytes = std::fs::read(ca_path)
            .map_err(|err| format!("failed to read ca_file '{ca_path}': {err}"))?;
        let cert = reqwest::Certificate::from_pem(&bytes)
            .map_err(|err| format!("failed to parse ca_file '{ca_path}': {err}"))?;
        builder = builder.add_root_certificate(cert);
    }
    builder
        .build()
        .map_err(|err| format!("failed to build OIDC HTTP client: {err}"))
}

fn peek_issuer(token: &str) -> Option<String> {
    let parts = split_jwt(token).ok()?;
    let value: Value = decode_segment(parts.claims).ok()?;
    value
        .get("iss")
        .and_then(|v| v.as_str())
        .map(|s| s.trim_end_matches('/').to_string())
}

struct JwtParts<'a> {
    header: &'a str,
    claims: &'a str,
    signature: &'a str,
}

fn split_jwt(token: &str) -> Result<JwtParts<'_>, OidcVerifyError> {
    let mut parts = token.split('.');
    let header = parts
        .next()
        .ok_or_else(|| OidcVerifyError::Malformed("missing header".to_string()))?;
    let claims = parts
        .next()
        .ok_or_else(|| OidcVerifyError::Malformed("missing claims".to_string()))?;
    let signature = parts
        .next()
        .ok_or_else(|| OidcVerifyError::Malformed("missing signature".to_string()))?;
    if parts.next().is_some() {
        return Err(OidcVerifyError::Malformed("too many segments".to_string()));
    }
    Ok(JwtParts {
        header,
        claims,
        signature,
    })
}

fn decode_segment<T: for<'de> Deserialize<'de>>(segment: &str) -> Result<T, OidcVerifyError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(segment)
        .map_err(|err| OidcVerifyError::Malformed(format!("segment not base64url: {err}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| OidcVerifyError::Malformed(format!("segment JSON invalid: {err}")))
}

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
    kid: String,
}

#[derive(Debug, Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<Value>,
}

fn parse_jwks(document: JwksDocument) -> Result<HashMap<String, JwksEntry>, OidcVerifyError> {
    let mut keys = HashMap::new();
    for raw in document.keys {
        let Some(kid) = raw.get("kid").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(alg) = raw.get("alg").and_then(|v| v.as_str()) else {
            continue;
        };
        let algorithm = match SupportedAlgorithm::parse(alg) {
            Ok(value) => value,
            Err(OidcVerifyError::UnsupportedAlgorithm(_)) => {
                // Skip unknown algorithms instead of failing the whole refresh;
                // some IdPs publish keys for algorithms we don't yet handle.
                continue;
            }
            Err(other) => return Err(other),
        };
        let key = match algorithm {
            SupportedAlgorithm::RS256 => parse_rsa_jwk(&raw)?,
            SupportedAlgorithm::EdDSA => parse_eddsa_jwk(&raw)?,
        };
        keys.insert(kid.to_string(), JwksEntry { algorithm, key });
    }
    Ok(keys)
}

fn parse_rsa_jwk(raw: &Value) -> Result<PKey<Public>, OidcVerifyError> {
    let n = raw
        .get("n")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OidcVerifyError::Claim("RSA JWK missing 'n'".to_string()))?;
    let e = raw
        .get("e")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OidcVerifyError::Claim("RSA JWK missing 'e'".to_string()))?;
    let n_bytes = URL_SAFE_NO_PAD
        .decode(n)
        .map_err(|err| OidcVerifyError::Claim(format!("RSA JWK 'n' not base64url: {err}")))?;
    let e_bytes = URL_SAFE_NO_PAD
        .decode(e)
        .map_err(|err| OidcVerifyError::Claim(format!("RSA JWK 'e' not base64url: {err}")))?;
    let n_bn = BigNum::from_slice(&n_bytes)
        .map_err(|err| OidcVerifyError::Claim(format!("RSA JWK 'n' is not a BIGNUM: {err}")))?;
    let e_bn = BigNum::from_slice(&e_bytes)
        .map_err(|err| OidcVerifyError::Claim(format!("RSA JWK 'e' is not a BIGNUM: {err}")))?;
    let rsa = Rsa::from_public_components(n_bn, e_bn).map_err(|err| {
        OidcVerifyError::Claim(format!("RSA public key reconstruction failed: {err}"))
    })?;
    PKey::from_rsa(rsa)
        .map_err(|err| OidcVerifyError::Claim(format!("RSA PKey wrap failed: {err}")))
}

fn parse_eddsa_jwk(raw: &Value) -> Result<PKey<Public>, OidcVerifyError> {
    let crv = raw
        .get("crv")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OidcVerifyError::Claim("OKP JWK missing 'crv'".to_string()))?;
    if crv != "Ed25519" {
        return Err(OidcVerifyError::UnsupportedAlgorithm(format!(
            "EdDSA curve {crv}"
        )));
    }
    let x = raw
        .get("x")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OidcVerifyError::Claim("OKP JWK missing 'x'".to_string()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(x)
        .map_err(|err| OidcVerifyError::Claim(format!("OKP JWK 'x' not base64url: {err}")))?;
    PKey::public_key_from_raw_bytes(&bytes, Id::ED25519)
        .map_err(|err| OidcVerifyError::Claim(format!("Ed25519 PKey decode failed: {err}")))
}

fn verify_signature(
    entry: &JwksEntry,
    data: &[u8],
    signature: &[u8],
) -> Result<(), OidcVerifyError> {
    let ok = match entry.algorithm {
        SupportedAlgorithm::RS256 => {
            let mut verifier =
                Verifier::new(MessageDigest::sha256(), &entry.key).map_err(|err| {
                    OidcVerifyError::Claim(format!("failed to initialize RS256 verifier: {err}"))
                })?;
            verifier
                .update(data)
                .map_err(|err| OidcVerifyError::Claim(format!("RS256 update failed: {err}")))?;
            verifier
                .verify(signature)
                .map_err(|err| OidcVerifyError::Claim(format!("RS256 verify failed: {err}")))?
        }
        SupportedAlgorithm::EdDSA => {
            let mut verifier = Verifier::new_without_digest(&entry.key).map_err(|err| {
                OidcVerifyError::Claim(format!("failed to initialize EdDSA verifier: {err}"))
            })?;
            verifier
                .update(data)
                .map_err(|err| OidcVerifyError::Claim(format!("EdDSA update failed: {err}")))?;
            verifier
                .verify(signature)
                .map_err(|err| OidcVerifyError::Claim(format!("EdDSA verify failed: {err}")))?
        }
    };
    if ok {
        Ok(())
    } else {
        Err(OidcVerifyError::InvalidSignature)
    }
}

fn validate_claims(provider: &OidcProvider, claims: &Value) -> Result<(), OidcVerifyError> {
    let iss = claims
        .get("iss")
        .and_then(|v| v.as_str())
        .ok_or_else(|| OidcVerifyError::Claim("missing 'iss'".to_string()))?;
    if iss.trim_end_matches('/') != provider.issuer_url {
        return Err(OidcVerifyError::Claim(
            "'iss' does not match configured issuer".to_string(),
        ));
    }
    let aud_ok = match claims.get("aud") {
        Some(Value::String(s)) => s == &provider.client_id,
        Some(Value::Array(items)) => items
            .iter()
            .any(|v| v.as_str() == Some(provider.client_id.as_str())),
        _ => false,
    };
    if !aud_ok {
        return Err(OidcVerifyError::Claim(
            "'aud' does not include the configured client_id".to_string(),
        ));
    }
    let now = unix_timestamp();
    if let Some(exp) = claims.get("exp").and_then(|v| v.as_u64()) {
        if now > exp.saturating_add(JWT_LEEWAY_SECONDS) {
            return Err(OidcVerifyError::Claim("token is expired".to_string()));
        }
    } else {
        return Err(OidcVerifyError::Claim("missing 'exp'".to_string()));
    }
    if let Some(nbf) = claims.get("nbf").and_then(|v| v.as_u64())
        && now.saturating_add(JWT_LEEWAY_SECONDS) < nbf
    {
        return Err(OidcVerifyError::Claim("token is not yet valid".to_string()));
    }
    if let Some(iat) = claims.get("iat").and_then(|v| v.as_u64())
        && now.saturating_add(JWT_LEEWAY_SECONDS) < iat
    {
        return Err(OidcVerifyError::Claim(
            "token was issued in the future".to_string(),
        ));
    }
    for (claim, expected) in &provider.required_claims {
        let actual = claims
            .get(claim)
            .and_then(|v| v.as_str())
            .ok_or_else(|| OidcVerifyError::Claim(format!("required claim {claim} missing")))?;
        if actual != expected {
            return Err(OidcVerifyError::Claim(format!(
                "required claim {claim} does not match expected value"
            )));
        }
    }
    Ok(())
}

fn build_identity(
    provider: &OidcProvider,
    claims: &Value,
) -> Result<VerifiedOidcIdentity, OidcVerifyError> {
    let raw_username = claims
        .get(&provider.username_claim)
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            OidcVerifyError::Claim(format!(
                "username claim '{}' is missing or not a string",
                provider.username_claim
            ))
        })?;
    let username = format!("{}{}", provider.username_prefix, raw_username);

    let groups = match claims.get(&provider.groups_claim) {
        None => Vec::new(),
        Some(Value::String(s)) => vec![apply_prefix(&provider.groups_prefix, s)],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| apply_prefix(&provider.groups_prefix, s))
            .collect(),
        Some(_) => {
            return Err(OidcVerifyError::Claim(format!(
                "groups claim '{}' must be a string or array of strings",
                provider.groups_claim
            )));
        }
    };

    Ok(VerifiedOidcIdentity {
        issuer: provider.issuer_url.clone(),
        username,
        groups,
    })
}

fn apply_prefix(prefix: &str, value: &str) -> String {
    if prefix.is_empty() {
        value.to_string()
    } else {
        format!("{prefix}{value}")
    }
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::sign::Signer;
    use serde_json::json;

    fn make_provider(issuer: &str, client_id: &str) -> OidcProvider {
        OidcProvider::from_config(&OidcProviderConfig {
            issuer_url: issuer.to_string(),
            client_id: client_id.to_string(),
            username_claim: "email".to_string(),
            username_prefix: "oidc:".to_string(),
            groups_claim: "groups".to_string(),
            groups_prefix: "oidc:".to_string(),
            required_claims: HashMap::new(),
            ca_file: None,
            jwks_refresh_seconds: 600,
            jwks_min_refresh_seconds: 30,
        })
        .expect("provider")
    }

    fn rsa_keypair_with_jwk(kid: &str) -> (PKey<openssl::pkey::Private>, JwksEntry) {
        let rsa = Rsa::generate(2048).expect("rsa");
        let private = PKey::from_rsa(rsa.clone()).expect("private");
        let public_pem = private.public_key_to_pem().expect("public pem");
        let public = PKey::public_key_from_pem(&public_pem).expect("public");
        let _ = kid;
        (
            private,
            JwksEntry {
                algorithm: SupportedAlgorithm::RS256,
                key: public,
            },
        )
    }

    fn encode_token(private: &PKey<openssl::pkey::Private>, kid: &str, claims: Value) -> String {
        let header = json!({"alg": "RS256", "kid": kid, "typ": "JWT"});
        let header_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
        let claims_b64 = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
        let signing_input = format!("{header_b64}.{claims_b64}");
        let mut signer = Signer::new(MessageDigest::sha256(), private).expect("signer");
        signer.update(signing_input.as_bytes()).expect("update");
        let signature = signer.sign_to_vec().expect("sign");
        format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
    }

    #[tokio::test]
    async fn verifies_well_formed_rs256_id_token() {
        let provider = make_provider("https://idp.example.com", "tugboat");
        let (private, entry) = rsa_keypair_with_jwk("kid-1");
        let mut keys = HashMap::new();
        keys.insert("kid-1".to_string(), entry);
        provider
            .seed_keys_for_test("https://idp.example.com/jwks", keys)
            .await;

        let now = unix_timestamp();
        let token = encode_token(
            &private,
            "kid-1",
            json!({
                "iss": "https://idp.example.com",
                "aud": "tugboat",
                "exp": now + 600,
                "iat": now,
                "nbf": now,
                "email": "alice@example.com",
                "groups": ["devs", "ops"],
            }),
        );

        let identity = provider.verify_token(&token).await.expect("verify");
        assert_eq!(identity.issuer, "https://idp.example.com");
        assert_eq!(identity.username, "oidc:alice@example.com");
        assert_eq!(identity.groups, vec!["oidc:devs", "oidc:ops"]);
    }

    #[tokio::test]
    async fn rejects_wrong_audience() {
        let provider = make_provider("https://idp.example.com", "tugboat");
        let (private, entry) = rsa_keypair_with_jwk("kid-1");
        let mut keys = HashMap::new();
        keys.insert("kid-1".to_string(), entry);
        provider
            .seed_keys_for_test("https://idp.example.com/jwks", keys)
            .await;

        let now = unix_timestamp();
        let token = encode_token(
            &private,
            "kid-1",
            json!({
                "iss": "https://idp.example.com",
                "aud": "someone-else",
                "exp": now + 600,
                "email": "alice@example.com",
            }),
        );

        let err = provider.verify_token(&token).await.expect_err("wrong aud");
        assert!(matches!(err, OidcVerifyError::Claim(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn rejects_expired_token() {
        let provider = make_provider("https://idp.example.com", "tugboat");
        let (private, entry) = rsa_keypair_with_jwk("kid-1");
        let mut keys = HashMap::new();
        keys.insert("kid-1".to_string(), entry);
        provider
            .seed_keys_for_test("https://idp.example.com/jwks", keys)
            .await;

        let now = unix_timestamp();
        let token = encode_token(
            &private,
            "kid-1",
            json!({
                "iss": "https://idp.example.com",
                "aud": "tugboat",
                "exp": now - 600,
                "email": "alice@example.com",
            }),
        );

        let err = provider.verify_token(&token).await.expect_err("expired");
        match err {
            OidcVerifyError::Claim(msg) => assert!(msg.contains("expired"), "got {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_required_claims_mismatch() {
        let mut provider = make_provider("https://idp.example.com", "tugboat");
        provider
            .required_claims
            .insert("hd".to_string(), "example.com".to_string());
        let (private, entry) = rsa_keypair_with_jwk("kid-1");
        let mut keys = HashMap::new();
        keys.insert("kid-1".to_string(), entry);
        provider
            .seed_keys_for_test("https://idp.example.com/jwks", keys)
            .await;

        let now = unix_timestamp();
        let token = encode_token(
            &private,
            "kid-1",
            json!({
                "iss": "https://idp.example.com",
                "aud": "tugboat",
                "exp": now + 600,
                "email": "alice@evil.example",
                "hd": "evil.example",
            }),
        );

        let err = provider
            .verify_token(&token)
            .await
            .expect_err("required claim mismatch");
        match err {
            OidcVerifyError::Claim(msg) => assert!(msg.contains("required claim hd"), "got {msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_unknown_kid_without_network() {
        // No keys seeded; refresh will attempt network and fail. We assert
        // that the error path returns a Jwks/Discovery error and never a
        // misleading InvalidSignature.
        let provider = make_provider("https://idp.example.invalid", "tugboat");
        let (private, _entry) = rsa_keypair_with_jwk("kid-1");
        let now = unix_timestamp();
        let token = encode_token(
            &private,
            "kid-1",
            json!({
                "iss": "https://idp.example.invalid",
                "aud": "tugboat",
                "exp": now + 600,
                "email": "alice@example.com",
            }),
        );
        let err = provider
            .verify_token(&token)
            .await
            .expect_err("no jwks available");
        assert!(
            matches!(
                err,
                OidcVerifyError::Discovery { .. } | OidcVerifyError::Jwks { .. }
            ),
            "got {err:?}",
        );
    }

    #[test]
    fn peek_issuer_reads_unverified_iss_claim() {
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"RS256\",\"kid\":\"k\"}");
        let claims =
            URL_SAFE_NO_PAD.encode(b"{\"iss\":\"https://idp.example.com/\",\"aud\":\"x\"}");
        let signature = URL_SAFE_NO_PAD.encode(b"not-a-signature");
        let token = format!("{header}.{claims}.{signature}");
        assert_eq!(
            peek_issuer(&token).as_deref(),
            Some("https://idp.example.com")
        );
    }

    #[test]
    fn peek_issuer_returns_none_for_non_jwt() {
        assert!(peek_issuer("not-a-jwt").is_none());
        assert!(peek_issuer("a.b").is_none());
    }

    #[test]
    fn provider_match_distinguishes_unknown_issuer() {
        let providers = vec![make_provider("https://idp.example.com", "tugboat")];
        let auth = OidcAuthenticator { providers };
        let header = URL_SAFE_NO_PAD.encode(b"{\"alg\":\"RS256\",\"kid\":\"k\"}");
        let claims = URL_SAFE_NO_PAD.encode(b"{\"iss\":\"https://other.example\"}");
        let signature = URL_SAFE_NO_PAD.encode(b"sig");
        let token = format!("{header}.{claims}.{signature}");
        match auth.provider_for_token(&token) {
            ProviderMatch::UnknownIssuer(iss) => assert_eq!(iss, "https://other.example"),
            other => panic!(
                "expected UnknownIssuer, got {}",
                provider_match_label(&other)
            ),
        }
    }

    fn provider_match_label(m: &ProviderMatch<'_>) -> &'static str {
        match m {
            ProviderMatch::Matched(_) => "Matched",
            ProviderMatch::UnknownIssuer(_) => "UnknownIssuer",
            ProviderMatch::NotJwt => "NotJwt",
        }
    }

    #[test]
    fn duplicate_issuer_detected_in_config() {
        let configs = vec![
            OidcProviderConfig {
                issuer_url: "https://idp.example.com".to_string(),
                client_id: "tugboat".to_string(),
                username_claim: "email".to_string(),
                username_prefix: "oidc:".to_string(),
                groups_claim: "groups".to_string(),
                groups_prefix: "oidc:".to_string(),
                required_claims: HashMap::new(),
                ca_file: None,
                jwks_refresh_seconds: 600,
                jwks_min_refresh_seconds: 30,
            },
            OidcProviderConfig {
                issuer_url: "https://idp.example.com/".to_string(),
                client_id: "tugboat".to_string(),
                username_claim: "email".to_string(),
                username_prefix: "oidc:".to_string(),
                groups_claim: "groups".to_string(),
                groups_prefix: "oidc:".to_string(),
                required_claims: HashMap::new(),
                ca_file: None,
                jwks_refresh_seconds: 600,
                jwks_min_refresh_seconds: 30,
            },
        ];
        let err = match OidcAuthenticator::from_config(&configs) {
            Err(e) => e,
            Ok(_) => panic!("expected duplicate-issuer error"),
        };
        assert!(err.contains("duplicate"), "got {err}");
    }

    #[test]
    fn parses_rsa_jwk_into_pkey() {
        let rsa = Rsa::generate(2048).expect("rsa");
        let n = URL_SAFE_NO_PAD.encode(rsa.n().to_vec());
        let e = URL_SAFE_NO_PAD.encode(rsa.e().to_vec());
        let raw = json!({"kty": "RSA", "kid": "k1", "alg": "RS256", "use": "sig", "n": n, "e": e});
        let pkey = parse_rsa_jwk(&raw).expect("pkey");
        assert_eq!(pkey.id(), Id::RSA);
    }
}
