# 14: tugboat-clientへの認証サポート追加

## 概要

`tugboat-client` (HTTPクライアント) に認証情報をリクエストに付加する機能を追加する。
scheduler, agent, controller-manager等がAPIサーバーに認証付きでアクセスできるようにする。

## 設計

### 認証設定

```rust
pub enum ClientAuth {
    None,
    BearerToken(String),
    ServiceAccountToken { token_path: String },
    ClientCertificate { cert_path: String, key_path: String },
}
```

### クライアント設定の拡張

既存のクライアント設定に認証設定を追加:

```toml
[apiserver]
url = "https://localhost:8443"

[apiserver.auth]
type = "bearer-token"
token = "..."
# or
# type = "service-account"
# token_path = "/var/run/secrets/tugboat.io/serviceaccount/token"
# or
# type = "client-certificate"
# cert_path = "/etc/tugboat/pki/client.crt"
# key_path = "/etc/tugboat/pki/client.key"
```

## 作業内容

1. `tugboat-client` に認証情報の保持・送信機能を追加
2. リクエストヘッダーに `Authorization: Bearer <token>` を自動付加
3. または TLS クライアント証明書を設定
4. 各コンポーネント (agent, scheduler, controller-manager) の設定にauth設定を追加

## 確認

```bash
cargo build --package tugboat-client
cargo build --package tugboat-agent
cargo build --package tugboat-scheduler
cargo build --package tugboat-controller-manager
```

## 参考

- 既存のtugboat-clientの実装
- 各コンポーネントの設定ファイル: `sample-configs/`
