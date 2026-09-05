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
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey};
use pkcs8::{
    DecodePrivateKey, DecodePublicKey, EncodePublicKey, LineEnding, ObjectIdentifier,
    PrivateKeyInfo,
};
use ring::rand::SystemRandom;
use ring::signature::{
    RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, RsaKeyPair, RsaPublicKeyComponents,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tugboat_resources::ObjectMetaResource;
use tugboat_resources::manifests::core::v1::ServiceAccount;
use tugboat_resources::manifests::meta::v1::Time;
use uuid::Uuid;

const DEFAULT_ISSUER: &str = "https://apiserver.tugboat.cloud";
const RSA_ENCRYPTION_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");
const RSA_ENCRYPTION_ALGORITHM_IDENTIFIER: &[u8] = &[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];
const RSA_ENCRYPTION_ALGORITHM_IDENTIFIER_WITHOUT_NULL: &[u8] = &[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01,
];

#[derive(Clone)]
pub(crate) struct ServiceAccountTokenIssuer {
    issuer: String,
    accepted_audiences: Vec<String>,
    default_token_ttl_seconds: u64,
    max_token_ttl_seconds: u64,
    leeway_seconds: u64,
    signing_key_id: String,
    signing_algorithm: JwtAlgorithm,
    signing_key: SigningKeyMaterial,
    verification_keys: HashMap<String, VerificationKey>,
}

#[derive(Clone)]
struct VerificationKey {
    algorithm: JwtAlgorithm,
    key: VerificationKeyMaterial,
    jwk: JsonWebKey,
}

#[derive(Clone)]
enum SigningKeyMaterial {
    Rsa(RsaSigningKey),
    Ed25519(SigningKey),
}

#[derive(Clone)]
enum VerificationKeyMaterial {
    Rsa(RsaPublicKey),
    Ed25519(VerifyingKey),
}

#[derive(Clone)]
struct RsaSigningKey {
    key_pair: Arc<RsaKeyPair>,
    public_key: RsaPublicKey,
}

#[derive(Clone)]
struct RsaPublicKey {
    modulus: Vec<u8>,
    exponent: Vec<u8>,
    pkcs1_der: Vec<u8>,
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
        let signing_key = parse_signing_key(&signing_pem)
            .map_err(|err| format!("failed to parse signing_key_file: {err}"))?;
        ensure_key_matches_algorithm(&signing_key, signing_algorithm)?;
        let public_key = verification_key_from_signing_key(&signing_key);
        let public_pem = public_key_to_pem(&public_key)
            .map_err(|err| format!("failed to derive public key from signing_key_file: {err}"))?;
        let signing_key_id = config
            .signing_key_id
            .clone()
            .unwrap_or_else(|| derive_key_id(&public_pem));

        let mut verification_keys = HashMap::from([(
            signing_key_id.clone(),
            VerificationKey::new(signing_key_id.clone(), signing_algorithm, public_key)?,
        )]);

        for (kid, path) in &config.additional_verification_keys {
            let pem = std::fs::read(path).map_err(|err| {
                format!("failed to read additional_verification_keys.{kid}: {err}")
            })?;
            let public_key = parse_public_key(&pem).map_err(|err| {
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
    fn new(
        kid: String,
        algorithm: JwtAlgorithm,
        key: VerificationKeyMaterial,
    ) -> Result<Self, String> {
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

fn sign(key: &SigningKeyMaterial, algorithm: JwtAlgorithm, data: &[u8]) -> Result<Vec<u8>, String> {
    match (algorithm, key) {
        (JwtAlgorithm::RS256, SigningKeyMaterial::Rsa(key)) => {
            let mut signature = vec![0; key.key_pair.public().modulus_len()];
            key.key_pair
                .sign(
                    &RSA_PKCS1_SHA256,
                    &SystemRandom::new(),
                    data,
                    &mut signature,
                )
                .map_err(|_| "failed to sign JWT".to_string())?;
            Ok(signature)
        }
        (JwtAlgorithm::EdDSA, SigningKeyMaterial::Ed25519(key)) => {
            Ok(key.sign(data).to_bytes().to_vec())
        }
        (JwtAlgorithm::RS256, _) => Err("RS256 signing requires an RSA private key".to_string()),
        (JwtAlgorithm::EdDSA, _) => {
            Err("EdDSA signing requires an Ed25519 private key".to_string())
        }
    }
}

fn verify_signature(
    key: &VerificationKeyMaterial,
    algorithm: JwtAlgorithm,
    data: &[u8],
    signature: &[u8],
) -> Result<(), String> {
    match (algorithm, key) {
        (JwtAlgorithm::RS256, VerificationKeyMaterial::Rsa(key)) => RsaPublicKeyComponents {
            n: &key.modulus,
            e: &key.exponent,
        }
        .verify(&RSA_PKCS1_2048_8192_SHA256, data, signature)
        .map_err(|_| "JWT signature is invalid".to_string()),
        (JwtAlgorithm::EdDSA, VerificationKeyMaterial::Ed25519(key)) => {
            let signature = Ed25519Signature::from_slice(signature)
                .map_err(|err| format!("JWT signature is invalid: {err}"))?;
            key.verify(data, &signature)
                .map_err(|_| "JWT signature is invalid".to_string())
        }
        (JwtAlgorithm::RS256, _) => {
            Err("RS256 verification requires an RSA public key".to_string())
        }
        (JwtAlgorithm::EdDSA, _) => {
            Err("EdDSA verification requires an Ed25519 public key".to_string())
        }
    }
}

fn parse_signing_key(bytes: &[u8]) -> Result<SigningKeyMaterial, String> {
    if let Some((label, der)) = decode_pem(bytes)? {
        return match label.as_str() {
            "PRIVATE KEY" => parse_pkcs8_signing_key(&der),
            "RSA PRIVATE KEY" => parse_rsa_signing_key(&der, false).map(SigningKeyMaterial::Rsa),
            _ => Err(format!("unsupported private key PEM label {label}")),
        };
    }
    if let Ok(key) = parse_rsa_signing_key(bytes, true) {
        return Ok(SigningKeyMaterial::Rsa(key));
    }
    if let Ok(key) = parse_rsa_signing_key(bytes, false) {
        return Ok(SigningKeyMaterial::Rsa(key));
    }
    if let Ok(key) = SigningKey::from_pkcs8_der(bytes) {
        return Ok(SigningKeyMaterial::Ed25519(key));
    }
    Err("unsupported private key PEM/DER; expected RSA PKCS#8/PKCS#1 or Ed25519 PKCS#8".to_string())
}

fn parse_public_key(bytes: &[u8]) -> Result<VerificationKeyMaterial, String> {
    if let Some((label, der)) = decode_pem(bytes)? {
        return match label.as_str() {
            "PUBLIC KEY" => parse_spki_public_key(&der),
            "RSA PUBLIC KEY" => parse_rsa_public_key(&der).map(VerificationKeyMaterial::Rsa),
            "PRIVATE KEY" => {
                parse_pkcs8_signing_key(&der).map(|key| verification_key_from_signing_key(&key))
            }
            "RSA PRIVATE KEY" => parse_rsa_signing_key(&der, false)
                .map(|key| VerificationKeyMaterial::Rsa(key.public_key)),
            _ => Err(format!("unsupported public key PEM label {label}")),
        };
    }
    if let Ok(key) = parse_rsa_spki_public_key(bytes) {
        return Ok(VerificationKeyMaterial::Rsa(key));
    }
    if let Ok(key) = parse_rsa_public_key(bytes) {
        return Ok(VerificationKeyMaterial::Rsa(key));
    }
    if let Ok(key) = VerifyingKey::from_public_key_der(bytes) {
        return Ok(VerificationKeyMaterial::Ed25519(key));
    }
    if let Ok(key) = parse_rsa_signing_key(bytes, true) {
        return Ok(VerificationKeyMaterial::Rsa(key.public_key));
    }
    if let Ok(key) = parse_rsa_signing_key(bytes, false) {
        return Ok(VerificationKeyMaterial::Rsa(key.public_key));
    }
    if let Ok(key) = SigningKey::from_pkcs8_der(bytes) {
        return Ok(VerificationKeyMaterial::Ed25519(key.verifying_key()));
    }
    Err(
        "unsupported public key PEM/DER; expected RSA SPKI/PKCS#1 or Ed25519 SPKI/private key"
            .to_string(),
    )
}

fn parse_pkcs8_signing_key(bytes: &[u8]) -> Result<SigningKeyMaterial, String> {
    if let Ok(key) = parse_rsa_signing_key(bytes, true) {
        return Ok(SigningKeyMaterial::Rsa(key));
    }
    SigningKey::from_pkcs8_der(bytes)
        .map(SigningKeyMaterial::Ed25519)
        .map_err(|_| "unsupported PKCS#8 private key; expected RSA or Ed25519".to_string())
}

fn parse_spki_public_key(bytes: &[u8]) -> Result<VerificationKeyMaterial, String> {
    if let Ok(key) = parse_rsa_spki_public_key(bytes) {
        return Ok(VerificationKeyMaterial::Rsa(key));
    }
    VerifyingKey::from_public_key_der(bytes)
        .map(VerificationKeyMaterial::Ed25519)
        .map_err(|_| "unsupported SPKI public key; expected RSA or Ed25519".to_string())
}

fn parse_rsa_signing_key(bytes: &[u8], pkcs8: bool) -> Result<RsaSigningKey, String> {
    let key_pair = if pkcs8 {
        match RsaKeyPair::from_pkcs8(bytes) {
            Ok(key_pair) => Ok(key_pair),
            Err(v1_error) => {
                let private_key_info = PrivateKeyInfo::try_from(bytes)
                    .map_err(|_| format!("invalid RSA private key: {v1_error}"))?;
                if private_key_info.algorithm.oid != RSA_ENCRYPTION_OID {
                    return Err("PKCS#8 private key is not RSA".to_string());
                }
                RsaKeyPair::from_der(private_key_info.private_key)
            }
        }
    } else {
        RsaKeyPair::from_der(bytes)
    }
    .map_err(|err| {
        format!(
            "invalid RSA private key (RS256 requires a 2048-4096-bit modulus and public exponent >= 65537): {err}"
        )
    })?;
    let public_key = parse_rsa_public_key(key_pair.public().as_ref())?;
    Ok(RsaSigningKey {
        key_pair: Arc::new(key_pair),
        public_key,
    })
}

fn parse_rsa_spki_public_key(bytes: &[u8]) -> Result<RsaPublicKey, String> {
    let mut outer = bytes;
    let sequence = read_der_value(&mut outer, 0x30)?;
    if !outer.is_empty() {
        return Err("RSA SPKI contains trailing data".to_string());
    }
    let mut sequence = sequence;
    let algorithm = read_der_value(&mut sequence, 0x30)?;
    if algorithm != RSA_ENCRYPTION_ALGORITHM_IDENTIFIER
        && algorithm != RSA_ENCRYPTION_ALGORITHM_IDENTIFIER_WITHOUT_NULL
    {
        return Err("SPKI does not contain an RSA public key".to_string());
    }
    let bit_string = read_der_value(&mut sequence, 0x03)?;
    if !sequence.is_empty() || bit_string.first() != Some(&0) {
        return Err("RSA SPKI bit string is invalid".to_string());
    }
    parse_rsa_public_key(&bit_string[1..])
}

fn parse_rsa_public_key(bytes: &[u8]) -> Result<RsaPublicKey, String> {
    let mut outer = bytes;
    let sequence = read_der_value(&mut outer, 0x30)?;
    if !outer.is_empty() {
        return Err("RSA public key contains trailing data".to_string());
    }
    let mut sequence = sequence;
    let modulus = read_positive_der_integer(&mut sequence)?;
    let exponent = read_positive_der_integer(&mut sequence)?;
    if !sequence.is_empty() {
        return Err("RSA public key contains unexpected fields".to_string());
    }
    validate_rsa_public_parameters(&modulus, &exponent)?;
    let pkcs1_der = encode_rsa_public_key(&modulus, &exponent);
    Ok(RsaPublicKey {
        modulus,
        exponent,
        pkcs1_der,
    })
}

fn validate_rsa_public_parameters(modulus: &[u8], exponent: &[u8]) -> Result<(), String> {
    let modulus_bits = modulus.len() * 8 - modulus[0].leading_zeros() as usize;
    if !(2048..=8192).contains(&modulus_bits) {
        return Err(format!(
            "RSA verification key modulus must be 2048-8192 bits, got {modulus_bits}"
        ));
    }
    if exponent.len() > 5 {
        return Err("RSA verification key exponent is out of range".to_string());
    }
    let exponent = exponent
        .iter()
        .fold(0u64, |value, byte| (value << 8) | u64::from(*byte));
    if !(3..=(1u64 << 33) - 1).contains(&exponent) || exponent.is_multiple_of(2) {
        return Err(
            "RSA verification key exponent must be an odd integer between 3 and 2^33-1".to_string(),
        );
    }
    Ok(())
}

fn read_der_value<'a>(input: &mut &'a [u8], expected_tag: u8) -> Result<&'a [u8], String> {
    let Some((&tag, rest)) = input.split_first() else {
        return Err("truncated DER value".to_string());
    };
    if tag != expected_tag {
        return Err("unexpected DER tag".to_string());
    }
    let Some((&first_length, rest)) = rest.split_first() else {
        return Err("truncated DER length".to_string());
    };
    let (length, rest) = if first_length & 0x80 == 0 {
        (usize::from(first_length), rest)
    } else {
        let length_bytes = usize::from(first_length & 0x7f);
        if length_bytes == 0
            || length_bytes > std::mem::size_of::<usize>()
            || length_bytes > rest.len()
        {
            return Err("invalid DER length".to_string());
        }
        let mut length = 0usize;
        for byte in &rest[..length_bytes] {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*byte)))
                .ok_or_else(|| "DER length is too large".to_string())?;
        }
        (length, &rest[length_bytes..])
    };
    if length > rest.len() {
        return Err("truncated DER value".to_string());
    }
    let (value, remaining) = rest.split_at(length);
    *input = remaining;
    Ok(value)
}

fn read_positive_der_integer(input: &mut &[u8]) -> Result<Vec<u8>, String> {
    let value = read_der_value(input, 0x02)?;
    if value.is_empty() || value[0] & 0x80 != 0 {
        return Err("RSA integer is not positive".to_string());
    }
    let value = value
        .iter()
        .position(|byte| *byte != 0)
        .map(|index| &value[index..])
        .ok_or_else(|| "RSA integer is zero".to_string())?;
    Ok(value.to_vec())
}

fn encode_rsa_public_key(modulus: &[u8], exponent: &[u8]) -> Vec<u8> {
    let mut body = encode_positive_der_integer(modulus);
    body.extend(encode_positive_der_integer(exponent));
    encode_der_value(0x30, &body)
}

fn encode_positive_der_integer(value: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(value.len() + 1);
    if value.first().is_some_and(|byte| byte & 0x80 != 0) {
        body.push(0);
    }
    body.extend_from_slice(value);
    encode_der_value(0x02, &body)
}

fn encode_der_value(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(value.len() + 6);
    encoded.push(tag);
    encode_der_length(value.len(), &mut encoded);
    encoded.extend_from_slice(value);
    encoded
}

fn encode_der_length(length: usize, output: &mut Vec<u8>) {
    if length < 128 {
        output.push(length as u8);
        return;
    }
    let bytes = length.to_be_bytes();
    let first = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    let bytes = &bytes[first..];
    output.push(0x80 | bytes.len() as u8);
    output.extend_from_slice(bytes);
}

fn decode_pem(bytes: &[u8]) -> Result<Option<(String, Vec<u8>)>, String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(None);
    };
    let mut lines = text.trim().lines();
    let Some(begin) = lines.next() else {
        return Ok(None);
    };
    let Some(label) = begin
        .strip_prefix("-----BEGIN ")
        .and_then(|line| line.strip_suffix("-----"))
    else {
        return Ok(None);
    };
    let end = format!("-----END {label}-----");
    let mut encoded = String::new();
    let mut found_end = false;
    for line in lines {
        if line.trim() == end {
            found_end = true;
            continue;
        }
        if found_end {
            if !line.trim().is_empty() {
                return Err("PEM contains data after the end marker".to_string());
            }
        } else {
            encoded.push_str(line.trim());
        }
    }
    if !found_end {
        return Err("PEM is missing its end marker".to_string());
    }
    STANDARD
        .decode(encoded)
        .map(|der| Some((label.to_string(), der)))
        .map_err(|err| format!("PEM base64 is invalid: {err}"))
}

fn ensure_key_matches_algorithm(
    key: &SigningKeyMaterial,
    algorithm: JwtAlgorithm,
) -> Result<(), String> {
    match (algorithm, key) {
        (JwtAlgorithm::RS256, SigningKeyMaterial::Rsa(_))
        | (JwtAlgorithm::EdDSA, SigningKeyMaterial::Ed25519(_)) => Ok(()),
        (JwtAlgorithm::RS256, _) => Err("RS256 signing requires an RSA private key".to_string()),
        (JwtAlgorithm::EdDSA, _) => {
            Err("EdDSA signing requires an Ed25519 private key".to_string())
        }
    }
}

fn verification_key_from_signing_key(key: &SigningKeyMaterial) -> VerificationKeyMaterial {
    match key {
        SigningKeyMaterial::Rsa(key) => VerificationKeyMaterial::Rsa(key.public_key.clone()),
        SigningKeyMaterial::Ed25519(key) => VerificationKeyMaterial::Ed25519(key.verifying_key()),
    }
}

fn ensure_public_key_matches_algorithm(
    key: &VerificationKeyMaterial,
    algorithm: JwtAlgorithm,
) -> Result<(), String> {
    match (algorithm, key) {
        (JwtAlgorithm::RS256, VerificationKeyMaterial::Rsa(_))
        | (JwtAlgorithm::EdDSA, VerificationKeyMaterial::Ed25519(_)) => Ok(()),
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
    key: &VerificationKeyMaterial,
) -> Result<JsonWebKey, String> {
    match (algorithm, key) {
        (JwtAlgorithm::RS256, VerificationKeyMaterial::Rsa(rsa)) => Ok(JsonWebKey {
            kty: "RSA".to_string(),
            kid: kid.to_string(),
            alg: algorithm.as_str().to_string(),
            key_use: "sig".to_string(),
            n: Some(URL_SAFE_NO_PAD.encode(&rsa.modulus)),
            e: Some(URL_SAFE_NO_PAD.encode(&rsa.exponent)),
            crv: None,
            x: None,
        }),
        (JwtAlgorithm::EdDSA, VerificationKeyMaterial::Ed25519(key)) => Ok(JsonWebKey {
            kty: "OKP".to_string(),
            kid: kid.to_string(),
            alg: algorithm.as_str().to_string(),
            key_use: "sig".to_string(),
            n: None,
            e: None,
            crv: Some("Ed25519".to_string()),
            x: Some(URL_SAFE_NO_PAD.encode(key.to_bytes())),
        }),
        (JwtAlgorithm::RS256, _) => {
            Err("RS256 verification requires an RSA public key".to_string())
        }
        (JwtAlgorithm::EdDSA, _) => {
            Err("EdDSA verification requires an Ed25519 public key".to_string())
        }
    }
}

fn detect_algorithm(key: &VerificationKeyMaterial) -> Result<JwtAlgorithm, String> {
    match key {
        VerificationKeyMaterial::Rsa(_) => Ok(JwtAlgorithm::RS256),
        VerificationKeyMaterial::Ed25519(_) => Ok(JwtAlgorithm::EdDSA),
    }
}

fn public_key_to_pem(key: &VerificationKeyMaterial) -> Result<Vec<u8>, String> {
    match key {
        VerificationKeyMaterial::Rsa(key) => {
            let mut bit_string = Vec::with_capacity(key.pkcs1_der.len() + 1);
            bit_string.push(0);
            bit_string.extend_from_slice(&key.pkcs1_der);
            let mut spki = encode_der_value(0x30, RSA_ENCRYPTION_ALGORITHM_IDENTIFIER);
            spki.extend(encode_der_value(0x03, &bit_string));
            Ok(encode_pem("PUBLIC KEY", &encode_der_value(0x30, &spki)))
        }
        VerificationKeyMaterial::Ed25519(key) => key
            .to_public_key_pem(LineEnding::LF)
            .map(|pem| pem.into_bytes())
            .map_err(|err| format!("{err}")),
    }
}

fn encode_pem(label: &str, der: &[u8]) -> Vec<u8> {
    let encoded = STANDARD.encode(der);
    let mut pem = format!("-----BEGIN {label}-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).expect("base64 is ASCII"));
        pem.push('\n');
    }
    pem.push_str(&format!("-----END {label}-----\n"));
    pem.into_bytes()
}

#[cfg(test)]
pub(super) fn rsa_test_key_pair() -> (Arc<RsaKeyPair>, Vec<u8>, Vec<u8>) {
    let der = STANDARD
        .decode(include_str!("../../../testdata/rsa-private-key.pkcs8.b64").trim())
        .expect("RSA test key base64");
    let SigningKeyMaterial::Rsa(key) = parse_signing_key(&der).expect("RSA test key") else {
        panic!("RSA fixture is not an RSA key");
    };
    (
        key.key_pair,
        key.public_key.modulus,
        key.public_key.exponent,
    )
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
    use pkcs8::der::Encode;
    use pkcs8::{EncodePrivateKey, EncodePublicKey};
    use std::fs;
    use tugboat_resources::manifests::meta::v1::ObjectMeta;

    const RSA_PRIVATE_KEY: &str = include_str!("../../../testdata/rsa-private-key.pkcs8.b64");
    const RSA_PKCS1_PRIVATE_KEY: &str = include_str!("../../../testdata/rsa-private-key.pkcs1.b64");

    fn decode_test_key(encoded: &str) -> Vec<u8> {
        STANDARD.decode(encoded.trim()).expect("test key base64")
    }

    fn rsa_private_key_pem() -> Vec<u8> {
        encode_pem("PRIVATE KEY", &decode_test_key(RSA_PRIVATE_KEY))
    }

    // Generated once with OpenSSL; this static public fixture keeps legacy
    // PEM and automatically derived key-id compatibility independent of
    // OpenSSL.
    const OPENSSL_RSA_PUBLIC_KEY: &str = r#"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvAGZg3HGRAVtj2LeKk7C
cFXUfuM2M9XdGWYaPh/sKNaytYfxZvFOHOu2tYZ2DsLZ8CiiZWV6fc5dYbCXxbR2
GvfCDc85jQitL1MnBU7I1MqOO79FfiY75VF8o5ObT7UPcYfvoJaIRmmSkUJcLSvY
dmIRT6UZWjWEtR6CvcxAue2u+pD06TYS+KN3tBpRq8djv8txvsGJXhpOH/qJkS6D
9vXu+rFb/No2Ld21YW7t0RMeuWVLsYRFHRfoNduw7KmDclHhZaCC2V2PYyuVn4NX
rrETZfVxEVgOUs/R9ZPFzCTTLPTppj35DLNha3SImDX2G4C+pqlcLR8a/5zTSWtq
VwIDAQAB
-----END PUBLIC KEY-----
"#;

    #[test]
    fn signs_and_verifies_rs256_service_account_token() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key_path = dir.path().join("sa.key");
        fs::write(&key_path, rsa_private_key_pem()).expect("write key");
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
    fn signs_and_verifies_eddsa_service_account_token() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = SigningKey::from_bytes(&[7u8; 32]);
        let key_path = dir.path().join("sa.key");
        fs::write(
            &key_path,
            key.to_pkcs8_pem(LineEnding::LF).expect("pem").as_bytes(),
        )
        .expect("write key");
        let issuer = ServiceAccountTokenIssuer::from_config(&ServiceAccountTokenConfig {
            issuer: Some("https://issuer.example".to_string()),
            audiences: vec!["https://issuer.example".to_string()],
            signing_key_file: Some(key_path.to_string_lossy().into_owned()),
            signing_key_id: Some("current".to_string()),
            signing_algorithm: ServiceAccountSigningAlgorithm::EdDsa,
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
        let key_path = dir.path().join("sa.key");
        fs::write(&key_path, rsa_private_key_pem()).expect("write key");
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
        let signing_path = dir.path().join("sa.key");
        fs::write(&signing_path, rsa_private_key_pem()).expect("write signing key");
        // Rotation candidate is Ed25519 — must be tagged EdDSA, not the
        // primary algorithm.
        let rotation_key = SigningKey::from_bytes(&[8u8; 32]);
        let rotation_path = dir.path().join("rotation.key");
        fs::write(
            &rotation_path,
            rotation_key
                .verifying_key()
                .to_public_key_pem(LineEnding::LF)
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

    #[test]
    fn parses_der_key_material() {
        let rsa_pkcs8 = decode_test_key(RSA_PRIVATE_KEY);
        let rsa_pkcs1 = decode_test_key(RSA_PKCS1_PRIVATE_KEY);
        let SigningKeyMaterial::Rsa(rsa) = parse_signing_key(&rsa_pkcs8).expect("RSA signing key")
        else {
            panic!("fixture is not RSA");
        };
        let rsa_public = {
            let mut bit_string = vec![0];
            bit_string.extend_from_slice(&rsa.public_key.pkcs1_der);
            let mut spki = encode_der_value(0x30, RSA_ENCRYPTION_ALGORITHM_IDENTIFIER);
            spki.extend(encode_der_value(0x03, &bit_string));
            encode_der_value(0x30, &spki)
        };

        assert!(matches!(
            parse_signing_key(&rsa_pkcs8),
            Ok(SigningKeyMaterial::Rsa(_))
        ));
        assert!(matches!(
            parse_signing_key(&rsa_pkcs1),
            Ok(SigningKeyMaterial::Rsa(_))
        ));
        assert!(matches!(
            parse_public_key(&rsa_public),
            Ok(VerificationKeyMaterial::Rsa(_))
        ));

        let private_key_info =
            PrivateKeyInfo::try_from(rsa_pkcs8.as_slice()).expect("PKCS#8 private key info");
        let rsa_pkcs8_v2 = PrivateKeyInfo {
            algorithm: private_key_info.algorithm,
            private_key: private_key_info.private_key,
            public_key: Some(&rsa.public_key.pkcs1_der),
        }
        .to_der()
        .expect("PKCS#8 v2 DER");
        assert!(matches!(
            parse_signing_key(&rsa_pkcs8_v2),
            Ok(SigningKeyMaterial::Rsa(_))
        ));

        let ed25519 = SigningKey::from_bytes(&[6u8; 32]);
        let ed25519_pkcs8 = ed25519.to_pkcs8_der().expect("Ed25519 PKCS#8 DER");
        let ed25519_public = ed25519
            .verifying_key()
            .to_public_key_der()
            .expect("Ed25519 SPKI DER");

        assert!(matches!(
            parse_signing_key(ed25519_pkcs8.as_bytes()),
            Ok(SigningKeyMaterial::Ed25519(_))
        ));
        assert!(matches!(
            parse_public_key(ed25519_public.as_bytes()),
            Ok(VerificationKeyMaterial::Ed25519(_))
        ));
    }

    #[test]
    fn preserves_kid_for_openssl_public_key_fixture() {
        let public_key = parse_public_key(OPENSSL_RSA_PUBLIC_KEY.as_bytes()).expect("RSA key");
        let public_pem = public_key_to_pem(&public_key).expect("public key PEM");

        assert_eq!(public_pem, OPENSSL_RSA_PUBLIC_KEY.as_bytes());
        assert_eq!(derive_key_id(&public_pem), "5k50kWfIY7KdmWrD");
    }

    #[test]
    fn rejects_rsa_public_key_with_unsupported_modulus_size() {
        let modulus = vec![0x80; 128];
        let public_key = encode_rsa_public_key(&modulus, &[1, 0, 1]);

        let err = match parse_rsa_public_key(&public_key) {
            Err(err) => err,
            Ok(_) => panic!("1024-bit RSA must be rejected"),
        };

        assert!(err.contains("2048-8192"));
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
