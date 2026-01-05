use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use oci_distribution::secrets::RegistryAuth;
use regex::Regex;
use std::collections::HashMap;
use std::env::home_dir;
use std::fs::File;
use std::sync::LazyLock;

#[derive(serde::Deserialize)]
struct DockerConfigJson {
    auths: HashMap<String, AuthContent>,
}

#[derive(serde::Deserialize)]
struct AuthContent {
    auth: String,
}

pub(crate) fn load_auth_or_anonymous(host: &str) -> RegistryAuth {
    load_auth(host).unwrap_or(RegistryAuth::Anonymous)
}

static HTTP_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^https?//([\w.]+)"#).unwrap());

fn load_auth(host: &str) -> Option<RegistryAuth> {
    let home = home_dir()?;
    let config = home.join(".docker/config.json");
    let file = File::open(config).ok()?;
    let config: DockerConfigJson = serde_json::from_reader(file).ok()?;
    let auths: HashMap<_, _> = config
        .auths
        .into_iter()
        .map(|(k, v)| (HTTP_REGEX.replace(&k, "$1").to_string(), v.auth))
        .collect();
    let auth = auths.get(host)?.as_str();
    let auth = String::from_utf8(BASE64_STANDARD.decode(auth).ok()?).ok()?;
    let (user, pw) = auth.split_once(":")?;
    Some(RegistryAuth::Basic(user.to_string(), pw.to_string()))
}
