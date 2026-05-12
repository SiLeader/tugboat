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

use crate::config::{ServiceAccountSigningAlgorithm, ServiceAccountTokenConfig};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use openssl::hash::MessageDigest;
use openssl::pkey::{Id, PKey, Private, Public};
use openssl::sign::{Signer, Verifier};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::ServiceAccount;
use tugboat_resources::manifests::meta::v1::Time;
use uuid::Uuid;

const DEFAULT_ISSUER: &str = "https://apiserver.tugboat.cloud";

#[derive(Clone)]
pub(crate) struct ServiceAccountTokenIssuer {
    issuer: String,
    accepted_audiences: Vec<String>,
    default_token_ttl_seconds: u64,
    max_token_ttl_seconds: u64,
    leeway_seconds: u64,
    signing_key_id: String,
    signing_algorithm: JwtAlgorithm,
    signing_key: PKey<Private>,
    verification_keys: HashMap<String, VerificationKey>,
}

#[derive(Clone)]
struct VerificationKey {
    algorithm: JwtAlgorithm,
    key: PKey<Public>,
    jwk: JsonWebKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum JwtAlgorithm {
    RS256,
    EdDSA,
}

impl JwtAlgorithm {
    fn as_str(self) -> &'static str {
        match self {
            Self::RS256 => "RS256",
            Self::EdDSA => "EdDSA",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServiceAccountTokenRequest {
    #[serde(default)]
    pub(crate) audiences: Vec<String>,
    pub(crate) expiration_seconds: Option<u64>,
    pub(crate) bound_object_ref: Option<BoundObjectReference>,
}

#[derive(Clone, Debug, Deserialize, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoundObjectReference {
    pub(crate) kind: String,
    pub(crate) api_version: String,
    pub(crate) name: String,
    pub(crate) uid: Option<String>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServiceAccountTokenResponse {
    pub(crate) token: String,
    pub(crate) expiration_timestamp: Time,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ServiceAccountJwtClaims {
    iss: String,
    sub: String,
    aud: Vec<String>,
    exp: u64,
    iat: u64,
    nbf: u64,
    jti: String,
    #[serde(rename = "serviceaccount.tugboat.cloud/name")]
    service_account_name: String,
    #[serde(rename = "serviceaccount.tugboat.cloud/namespace")]
    service_account_namespace: String,
    #[serde(rename = "serviceaccount.tugboat.cloud/uid")]
    service_account_uid: Option<String>,
    #[serde(
        rename = "serviceaccount.tugboat.cloud/bound_object_ref",
        skip_serializing_if = "Option::is_none"
    )]
    bound_object_ref: Option<BoundObjectReference>,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedServiceAccountJwt {
    pub(crate) namespace: String,
    pub(crate) name: String,
    pub(crate) uid: Option<String>,
    pub(crate) bound_object_ref: Option<BoundObjectReference>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct OpenIdConfiguration {
    issuer: String,
    jwks_uri: String,
    response_types_supported: Vec<String>,
    subject_types_supported: Vec<String>,
    id_token_signing_alg_values_supported: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct JsonWebKeySet {
    keys: Vec<JsonWebKey>,
}

#[derive(Clone, Debug, Serialize)]
struct JsonWebKey {
    kty: String,
    kid: String,
    alg: String,
    #[serde(rename = "use")]
    key_use: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    e: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    crv: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    x: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct JwtHeader {
    alg: JwtAlgorithm,
    kid: String,
    typ: String,
}

impl ServiceAccountTokenIssuer {
    pub(crate) fn from_config(config: &ServiceAccountTokenConfig) -> Result<Option<Self>, String> {
        let Some(signing_key_file) = &config.signing_key_file else {
            return Ok(None);
        };
        let issuer = config
            .issuer
            .clone()
            .unwrap_or_else(|| DEFAULT_ISSUER.to_string());
        let accepted_audiences = if config.audiences.is_empty() {
            vec![issuer.clone()]
        } else {
            config.audiences.clone()
        };
        let signing_algorithm = match config.signing_algorithm {
            ServiceAccountSigningAlgorithm::RS256 => JwtAlgorithm::RS256,
            ServiceAccountSigningAlgorithm::EdDsa => JwtAlgorithm::EdDSA,
        };
        let signing_pem = std::fs::read(signing_key_file)
            .map_err(|err| format!("failed to read signing_key_file: {err}"))?;
        let signing_key = PKey::private_key_from_pem(&signing_pem)
            .map_err(|err| format!("failed to parse signing_key_file: {err}"))?;
        ensure_key_matches_algorithm(&signing_key, signing_algorithm)?;
        let public_pem = signing_key
            .public_key_to_pem()
            .map_err(|err| format!("failed to derive public key from signing_key_file: {err}"))?;
        let signing_key_id = config
            .signing_key_id
            .clone()
            .unwrap_or_else(|| derive_key_id(&public_pem));

        let public_key = PKey::public_key_from_pem(&public_pem)
            .map_err(|err| format!("failed to parse derived public key: {err}"))?;
        let mut verification_keys = HashMap::from([(
            signing_key_id.clone(),
            VerificationKey::new(signing_key_id.clone(), signing_algorithm, public_key)?,
        )]);

        for (kid, path) in &config.additional_verification_keys {
            let pem = std::fs::read(path).map_err(|err| {
                format!("failed to read additional_verification_keys.{kid}: {err}")
            })?;
            let public_key = parse_public_key_pem(&pem).map_err(|err| {
                format!("failed to parse additional_verification_keys.{kid}: {err}")
            })?;
            let algorithm = detect_algorithm(&public_key)
                .map_err(|err| format!("additional_verification_keys.{kid}: {err}"))?;
            verification_keys.insert(
                kid.clone(),
                VerificationKey::new(kid.clone(), algorithm, public_key)?,
            );
        }

        Ok(Some(Self {
            issuer,
            accepted_audiences,
            default_token_ttl_seconds: config.default_token_ttl_seconds,
            max_token_ttl_seconds: config.max_token_ttl_seconds,
            leeway_seconds: config.leeway_seconds,
            signing_key_id,
            signing_algorithm,
            signing_key,
            verification_keys,
        }))
    }

    pub(crate) fn issuer(&self) -> &str {
        &self.issuer
    }

    pub(crate) fn issue_token(
        &self,
        service_account: &ServiceAccount,
        request: ServiceAccountTokenRequest,
    ) -> Result<ServiceAccountTokenResponse, String> {
        let namespace = service_account
            .namespace()
            .ok_or_else(|| "ServiceAccount is missing metadata.namespace".to_string())?;
        let name = service_account
            .name()
            .ok_or_else(|| "ServiceAccount is missing metadata.name".to_string())?;
        let uid = service_account
            .object_meta()
            .as_ref()
            .and_then(|meta| meta.uid.clone());
        let ttl = request
            .expiration_seconds
            .unwrap_or(self.default_token_ttl_seconds)
            .min(self.max_token_ttl_seconds);
        if ttl == 0 {
            return Err("expirationSeconds must be greater than zero".to_string());
        }
        let now = unix_timestamp();
        let exp = now.saturating_add(ttl);
        let audiences = if request.audiences.is_empty() {
            self.accepted_audiences.clone()
        } else {
            request.audiences
        };
        let claims = ServiceAccountJwtClaims {
            iss: self.issuer.clone(),
            sub: service_account_subject(namespace, name),
            aud: audiences,
            exp,
            iat: now,
            nbf: now,
            jti: Uuid::new_v4().to_string(),
            service_account_name: name.to_string(),
            service_account_namespace: namespace.to_string(),
            service_account_uid: uid,
            bound_object_ref: request.bound_object_ref,
        };
        let token = self.encode_claims(&claims)?;
        Ok(ServiceAccountTokenResponse {
            token,
            expiration_timestamp: Time {
                seconds: exp as i64,
                nanos: 0,
            },
        })
    }

    pub(crate) fn verify_token(&self, token: &str) -> Result<VerifiedServiceAccountJwt, String> {
        let (header_b64, claims_b64, signature_b64) = split_compact_jwt(token)?;
        let header: JwtHeader = decode_json(header_b64)?;
        let Some(key) = self.verification_keys.get(&header.kid) else {
            return Err("JWT kid is not trusted".to_string());
        };
        if header.alg != key.algorithm {
            return Err("JWT alg does not match verification key".to_string());
        }
        let signing_input = format!("{header_b64}.{claims_b64}");
        let signature = URL_SAFE_NO_PAD
            .decode(signature_b64)
            .map_err(|err| format!("JWT signature is not base64url: {err}"))?;
        verify_signature(
            &key.key,
            key.algorithm,
            signing_input.as_bytes(),
            &signature,
        )?;

        let claims: ServiceAccountJwtClaims = decode_json(claims_b64)?;
        self.validate_claims(claims)
    }

    pub(crate) fn jwks(&self) -> JsonWebKeySet {
        let mut keys = self
            .verification_keys
            .values()
            .map(|key| key.jwk.clone())
            .collect::<Vec<_>>();
        keys.sort_by(|a, b| a.kid.cmp(&b.kid));
        JsonWebKeySet { keys }
    }

    pub(crate) fn openid_configuration(&self) -> OpenIdConfiguration {
        let mut algorithms: Vec<&'static str> = self
            .verification_keys
            .values()
            .map(|key| key.algorithm.as_str())
            .collect();
        algorithms.sort_unstable();
        algorithms.dedup();
        OpenIdConfiguration {
            issuer: self.issuer.clone(),
            jwks_uri: format!("{}/openid/v1/jwks", self.issuer.trim_end_matches('/')),
            response_types_supported: vec!["id_token".to_string()],
            subject_types_supported: vec!["public".to_string()],
            id_token_signing_alg_values_supported: algorithms
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }

    fn encode_claims(&self, claims: &ServiceAccountJwtClaims) -> Result<String, String> {
        let header = JwtHeader {
            alg: self.signing_algorithm,
            kid: self.signing_key_id.clone(),
            typ: "JWT".to_string(),
        };
        let header_b64 = encode_json(&header)?;
        let claims_b64 = encode_json(claims)?;
        let signing_input = format!("{header_b64}.{claims_b64}");
        let signature = sign(
            &self.signing_key,
            self.signing_algorithm,
            signing_input.as_bytes(),
        )?;
        Ok(format!(
            "{signing_input}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    fn validate_claims(
        &self,
        claims: ServiceAccountJwtClaims,
    ) -> Result<VerifiedServiceAccountJwt, String> {
        if claims.iss != self.issuer {
            return Err("JWT issuer is not accepted".to_string());
        }
        if !claims.aud.iter().any(|audience| {
            self.accepted_audiences
                .iter()
                .any(|accepted| accepted == audience)
        }) {
            return Err("JWT audience is not accepted".to_string());
        }

        let now = unix_timestamp();
        if now > claims.exp.saturating_add(self.leeway_seconds) {
            return Err("JWT is expired".to_string());
        }
        if now.saturating_add(self.leeway_seconds) < claims.nbf {
            return Err("JWT is not valid yet".to_string());
        }
        if now.saturating_add(self.leeway_seconds) < claims.iat {
            return Err("JWT was issued in the future".to_string());
        }

        let expected_subject = service_account_subject(
            &claims.service_account_namespace,
            &claims.service_account_name,
        );
        if claims.sub != expected_subject {
            return Err("JWT subject does not match service account claims".to_string());
        }

        Ok(VerifiedServiceAccountJwt {
            namespace: claims.service_account_namespace,
            name: claims.service_account_name,
            uid: claims.service_account_uid,
            bound_object_ref: claims.bound_object_ref,
        })
    }
}

impl VerificationKey {
    fn new(kid: String, algorithm: JwtAlgorithm, key: PKey<Public>) -> Result<Self, String> {
        ensure_public_key_matches_algorithm(&key, algorithm)?;
        let jwk = jwk_from_public_key(&kid, algorithm, &key)?;
        Ok(Self {
            algorithm,
            key,
            jwk,
        })
    }
}

pub(crate) fn looks_like_jwt(token: &str) -> bool {
    token.split('.').count() == 3
}

fn service_account_subject(namespace: &str, name: &str) -> String {
    format!("system:serviceaccount:{namespace}:{name}")
}

fn split_compact_jwt(token: &str) -> Result<(&str, &str, &str), String> {
    let mut parts = token.split('.');
    let header = parts
        .next()
        .ok_or_else(|| "JWT is missing header".to_string())?;
    let claims = parts
        .next()
        .ok_or_else(|| "JWT is missing claims".to_string())?;
    let signature = parts
        .next()
        .ok_or_else(|| "JWT is missing signature".to_string())?;
    if parts.next().is_some() {
        return Err("JWT has too many segments".to_string());
    }
    Ok((header, claims, signature))
}

fn encode_json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
        .map_err(|err| format!("failed to serialize JWT JSON: {err}"))
}

fn decode_json<T: for<'de> Deserialize<'de>>(value: &str) -> Result<T, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|err| format!("JWT segment is not base64url: {err}"))?;
    serde_json::from_slice(&bytes).map_err(|err| format!("JWT JSON is invalid: {err}"))
}

fn sign(key: &PKey<Private>, algorithm: JwtAlgorithm, data: &[u8]) -> Result<Vec<u8>, String> {
    match algorithm {
        JwtAlgorithm::RS256 => {
            let mut signer = Signer::new(MessageDigest::sha256(), key)
                .map_err(|err| format!("failed to initialize RS256 signer: {err}"))?;
            signer
                .update(data)
                .map_err(|err| format!("failed to feed RS256 signer: {err}"))?;
            signer
                .sign_to_vec()
                .map_err(|err| format!("failed to sign JWT: {err}"))
        }
        JwtAlgorithm::EdDSA => {
            let mut signer = Signer::new_without_digest(key)
                .map_err(|err| format!("failed to initialize EdDSA signer: {err}"))?;
            signer
                .update(data)
                .map_err(|err| format!("failed to feed EdDSA signer: {err}"))?;
            signer
                .sign_to_vec()
                .map_err(|err| format!("failed to sign JWT: {err}"))
        }
    }
}

fn verify_signature(
    key: &PKey<Public>,
    algorithm: JwtAlgorithm,
    data: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    let verified = match algorithm {
        JwtAlgorithm::RS256 => {
            let mut verifier = Verifier::new(MessageDigest::sha256(), key)
                .map_err(|err| format!("failed to initialize RS256 verifier: {err}"))?;
            verifier
                .update(data)
                .map_err(|err| format!("failed to feed RS256 verifier: {err}"))?;
            verifier
                .verify(signature)
                .map_err(|err| format!("failed to verify JWT: {err}"))?
        }
        JwtAlgorithm::EdDSA => {
            let mut verifier = Verifier::new_without_digest(key)
                .map_err(|err| format!("failed to initialize EdDSA verifier: {err}"))?;
            verifier
                .update(data)
                .map_err(|err| format!("failed to feed EdDSA verifier: {err}"))?;
            verifier
                .verify(signature)
                .map_err(|err| format!("failed to verify JWT: {err}"))?
        }
    };
    if verified {
        Ok(())
    } else {
        Err("JWT signature is invalid".to_string())
    }
}

fn parse_public_key_pem(pem: &[u8]) -> Result<PKey<Public>, openssl::error::ErrorStack> {
    if let Ok(key) = PKey::public_key_from_pem(pem) {
        return Ok(key);
    }
    PKey::private_key_from_pem(pem).and_then(|key| {
        let public_pem = key.public_key_to_pem()?;
        PKey::public_key_from_pem(&public_pem)
    })
}

fn ensure_key_matches_algorithm(
    key: &PKey<Private>,
    algorithm: JwtAlgorithm,
) -> Result<(), String> {
    let id = key.id();
    match (algorithm, id) {
        (JwtAlgorithm::RS256, Id::RSA) | (JwtAlgorithm::EdDSA, Id::ED25519) => Ok(()),
        (JwtAlgorithm::RS256, _) => Err("RS256 signing requires an RSA private key".to_string()),
        (JwtAlgorithm::EdDSA, _) => {
            Err("EdDSA signing requires an Ed25519 private key".to_string())
        }
    }
}

fn detect_algorithm(key: &PKey<Public>) -> Result<JwtAlgorithm, String> {
    match key.id() {
        Id::RSA => Ok(JwtAlgorithm::RS256),
        Id::ED25519 => Ok(JwtAlgorithm::EdDSA),
        other => Err(format!(
            "unsupported public key type {other:?}; expected RSA or Ed25519"
        )),
    }
}

fn ensure_public_key_matches_algorithm(
    key: &PKey<Public>,
    algorithm: JwtAlgorithm,
) -> Result<(), String> {
    let id = key.id();
    match (algorithm, id) {
        (JwtAlgorithm::RS256, Id::RSA) | (JwtAlgorithm::EdDSA, Id::ED25519) => Ok(()),
        (JwtAlgorithm::RS256, _) => {
            Err("RS256 verification requires an RSA public key".to_string())
        }
        (JwtAlgorithm::EdDSA, _) => {
            Err("EdDSA verification requires an Ed25519 public key".to_string())
        }
    }
}

fn jwk_from_public_key(
    kid: &str,
    algorithm: JwtAlgorithm,
    key: &PKey<Public>,
) -> Result<JsonWebKey, String> {
    match algorithm {
        JwtAlgorithm::RS256 => {
            let rsa = key
                .rsa()
                .map_err(|err| format!("failed to extract RSA public key: {err}"))?;
            Ok(JsonWebKey {
                kty: "RSA".to_string(),
                kid: kid.to_string(),
                alg: algorithm.as_str().to_string(),
                key_use: "sig".to_string(),
                n: Some(URL_SAFE_NO_PAD.encode(rsa.n().to_vec())),
                e: Some(URL_SAFE_NO_PAD.encode(rsa.e().to_vec())),
                crv: None,
                x: None,
            })
        }
        JwtAlgorithm::EdDSA => Ok(JsonWebKey {
            kty: "OKP".to_string(),
            kid: kid.to_string(),
            alg: algorithm.as_str().to_string(),
            key_use: "sig".to_string(),
            n: None,
            e: None,
            crv: Some("Ed25519".to_string()),
            x: Some(
                URL_SAFE_NO_PAD.encode(
                    key.raw_public_key()
                        .map_err(|err| format!("failed to extract Ed25519 public key: {err}"))?,
                ),
            ),
        }),
    }
}

fn derive_key_id(public_pem: &[u8]) -> String {
    let digest = Sha256::digest(public_pem);
    URL_SAFE_NO_PAD.encode(&digest[..12])
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
    use openssl::rsa::Rsa;
    use std::fs;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    #[test]
    fn signs_and_verifies_rs256_service_account_token() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = PKey::from_rsa(Rsa::generate(2048).expect("rsa")).expect("pkey");
        let key_path = dir.path().join("sa.key");
        fs::write(&key_path, key.private_key_to_pem_pkcs8().expect("pem")).expect("write key");
        let issuer = ServiceAccountTokenIssuer::from_config(&ServiceAccountTokenConfig {
            issuer: Some("https://issuer.example".to_string()),
            audiences: vec!["https://issuer.example".to_string()],
            signing_key_file: Some(key_path.to_string_lossy().into_owned()),
            signing_key_id: Some("current".to_string()),
            ..Default::default()
        })
        .expect("config")
        .expect("issuer");
        let service_account = service_account("default", "builder", "sa-uid");

        let response = issuer
            .issue_token(
                &service_account,
                ServiceAccountTokenRequest {
                    audiences: Vec::new(),
                    expiration_seconds: Some(600),
                    bound_object_ref: None,
                },
            )
            .expect("token");
        let verified = issuer.verify_token(&response.token).expect("verified");

        assert_eq!(verified.namespace, "default");
        assert_eq!(verified.name, "builder");
        assert_eq!(verified.uid.as_deref(), Some("sa-uid"));
    }

    #[test]
    fn rejects_wrong_audience() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = PKey::from_rsa(Rsa::generate(2048).expect("rsa")).expect("pkey");
        let key_path = dir.path().join("sa.key");
        fs::write(&key_path, key.private_key_to_pem_pkcs8().expect("pem")).expect("write key");
        let issuer = ServiceAccountTokenIssuer::from_config(&ServiceAccountTokenConfig {
            issuer: Some("https://issuer.example".to_string()),
            audiences: vec!["https://issuer.example".to_string()],
            signing_key_file: Some(key_path.to_string_lossy().into_owned()),
            signing_key_id: Some("current".to_string()),
            ..Default::default()
        })
        .expect("config")
        .expect("issuer");
        let response = issuer
            .issue_token(
                &service_account("default", "builder", "sa-uid"),
                ServiceAccountTokenRequest {
                    audiences: vec!["external".to_string()],
                    expiration_seconds: Some(600),
                    bound_object_ref: None,
                },
            )
            .expect("token");

        let err = issuer
            .verify_token(&response.token)
            .expect_err("wrong audience should fail");

        assert!(err.contains("audience"));
    }

    #[test]
    fn additional_verification_key_uses_its_own_algorithm() {
        let dir = tempfile::tempdir().expect("temp dir");
        // Primary signing key is RS256.
        let signing_key = PKey::from_rsa(Rsa::generate(2048).expect("rsa")).expect("pkey");
        let signing_path = dir.path().join("sa.key");
        fs::write(
            &signing_path,
            signing_key.private_key_to_pem_pkcs8().expect("pem"),
        )
        .expect("write signing key");
        // Rotation candidate is Ed25519 — must be tagged EdDSA, not the
        // primary algorithm.
        let rotation_key = PKey::generate_ed25519().expect("ed25519");
        let rotation_path = dir.path().join("rotation.key");
        fs::write(
            &rotation_path,
            rotation_key
                .public_key_to_pem()
                .expect("rotation public pem"),
        )
        .expect("write rotation key");

        let issuer = ServiceAccountTokenIssuer::from_config(&ServiceAccountTokenConfig {
            issuer: Some("https://issuer.example".to_string()),
            audiences: vec!["https://issuer.example".to_string()],
            signing_key_file: Some(signing_path.to_string_lossy().into_owned()),
            signing_key_id: Some("primary".to_string()),
            additional_verification_keys: HashMap::from([(
                "rotation".to_string(),
                rotation_path.to_string_lossy().into_owned(),
            )]),
            ..Default::default()
        })
        .expect("config")
        .expect("issuer");

        let rotation_alg = issuer
            .verification_keys
            .get("rotation")
            .expect("rotation key registered")
            .algorithm;
        let primary_alg = issuer
            .verification_keys
            .get("primary")
            .expect("primary key registered")
            .algorithm;
        assert_eq!(primary_alg, JwtAlgorithm::RS256);
        // The rotation key is Ed25519 — it must be tagged EdDSA, not the
        // primary signing algorithm.
        assert_eq!(rotation_alg, JwtAlgorithm::EdDSA);
    }

    fn service_account(namespace: &str, name: &str, uid: &str) -> ServiceAccount {
        ServiceAccount {
            object_meta: Some(ObjectMeta {
                namespace: Some(namespace.to_string()),
                name: Some(name.to_string()),
                uid: Some(uid.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }
    }
}
