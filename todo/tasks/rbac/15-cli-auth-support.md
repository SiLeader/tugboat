# 15: tugboat-cli への認証サポート追加

## 概要

`tugboat-cli` (CLIツール) にユーザー認証の設定・管理機能を追加する。
kubectlのkubeconfigに相当する設定ファイルでコンテキスト・認証情報を管理する。

## 設計

### 設定ファイル (~/.tugboat/config)

```toml
[current-context]
name = "default"

[[contexts]]
name = "default"
cluster = "local"
user = "admin"
namespace = "default"

[[clusters]]
name = "local"
server = "https://localhost:8443"
certificate_authority = "/etc/tugboat/pki/ca.crt"
# insecure_skip_tls_verify = false

[[users]]
name = "admin"
# 方式1: Bearer Token
token = "..."
# 方式2: クライアント証明書
# client_certificate = "/etc/tugboat/pki/admin.crt"
# client_key = "/etc/tugboat/pki/admin.key"
```

### サブコマンド

- `tugboat config set-context <name> --cluster=<cluster> --user=<user> --namespace=<ns>`
- `tugboat config use-context <name>`
- `tugboat config set-credentials <name> --token=<token>`
- `tugboat config set-cluster <name> --server=<url>`
- `tugboat config view`

## 作業内容

1. 設定ファイルの構造体定義と読み書きロジック
2. `tugboat config` サブコマンド群の実装
3. 既存のCLI操作で認証情報を利用するよう統合
4. `--token`, `--as` などのグローバルフラグ追加

## 確認

```bash
cargo build --package tugboat-cli
cargo test --package tugboat-cli
```

## 参考

- kubectl の kubeconfig 構造
- 既存のtugboat-cliの実装
