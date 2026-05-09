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

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use oci_distribution::secrets::RegistryAuth;
use std::collections::HashMap;
use std::env::home_dir;
use std::fs::File;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerConfigJson {
    #[serde(default)]
    auths: HashMap<String, AuthContent>,
    creds_store: Option<String>,
    #[serde(default)]
    cred_helpers: HashMap<String, String>,
}

#[derive(serde::Deserialize)]
struct AuthContent {
    auth: Option<String>,
}

pub(crate) async fn load_auth_or_anonymous(host: &str) -> RegistryAuth {
    load_auth(host).await.unwrap_or(RegistryAuth::Anonymous)
}

async fn load_auth(host: &str) -> Option<RegistryAuth> {
    let home = home_dir()?;
    let config = home.join(".docker/config.json");
    let file = File::open(config).ok()?;
    let config: DockerConfigJson = serde_json::from_reader(file).ok()?;

    // 1. Check credHelpers
    let cred_helpers: HashMap<_, _> = config
        .cred_helpers
        .into_iter()
        .map(|(k, v)| (docker_registry_key_host(&k), v))
        .collect();
    if let Some(helper) = cred_helpers.get(host)
        && let Some(auth) = call_helper(helper, host).await
    {
        return Some(auth);
    }

    // 2. Check credsStore
    if let Some(helper) = &config.creds_store
        && let Some(auth) = call_helper(helper, host).await
    {
        return Some(auth);
    }

    // 3. Check auths
    let auths: HashMap<_, _> = config
        .auths
        .into_iter()
        .map(|(k, v)| (docker_registry_key_host(&k), v.auth))
        .collect();
    if let Some(Some(auth)) = auths.get(host) {
        let auth = String::from_utf8(BASE64_STANDARD.decode(auth).ok()?).ok()?;
        let (user, pw) = auth.split_once(":")?;
        return Some(RegistryAuth::Basic(user.to_string(), pw.to_string()));
    }

    None
}

fn docker_registry_key_host(key: &str) -> String {
    let without_scheme = key
        .strip_prefix("https://")
        .or_else(|| key.strip_prefix("http://"))
        .unwrap_or(key);
    without_scheme
        .split('/')
        .next()
        .unwrap_or(without_scheme)
        .to_string()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HelperOutput {
    username: String,
    secret: String,
}

async fn call_helper(helper: &str, host: &str) -> Option<RegistryAuth> {
    use std::process::Stdio;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::process::Command;
    use tokio::time::{Duration, timeout};

    const HELPER_TIMEOUT: Duration = Duration::from_secs(30);

    let mut child = Command::new(format!("docker-credential-{}", helper))
        .arg("get")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;

    let mut stdin = child.stdin.take()?;
    let mut stdout = child.stdout.take()?;
    stdin.write_all(host.as_bytes()).await.ok()?;
    drop(stdin);

    let stdout_task = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes).await;
        bytes
    });

    let status = match timeout(HELPER_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(_)) => return None,
        Err(_) => {
            let _ = child.kill().await;
            return None;
        }
    };

    if !status.success() {
        return None;
    }

    let stdout = stdout_task.await.ok()?;
    let res: HelperOutput = serde_json::from_slice(&stdout).ok()?;
    Some(RegistryAuth::Basic(res.username, res.secret))
}

#[cfg(test)]
mod tests {
    use super::docker_registry_key_host;

    #[test]
    fn docker_registry_key_host_strips_scheme_and_path() {
        assert_eq!(
            docker_registry_key_host("https://index.docker.io/v1/"),
            "index.docker.io"
        );
        assert_eq!(
            docker_registry_key_host("http://localhost:5000/v2"),
            "localhost:5000"
        );
        assert_eq!(docker_registry_key_host("ghcr.io"), "ghcr.io");
    }
}
