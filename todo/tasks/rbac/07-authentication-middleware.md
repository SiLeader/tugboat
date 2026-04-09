# 07: 認証ミドルウェア (Authentication)

## 概要

APIサーバーへのリクエストから認証情報を抽出し、リクエストにユーザーIDを付加するミドルウェアを実装する。

## 設計

### UserInfo構造体

```rust
pub struct UserInfo {
    pub username: String,
    pub uid: Option<String>,
    pub groups: Vec<String>,
    pub extra: HashMap<String, Vec<String>>,
}
```

### 認証方式

以下の認証方式を段階的にサポート:

1. **ServiceAccountトークン認証** (Bearer Token)
   - `Authorization: Bearer <token>` ヘッダーからトークンを抽出
   - トークンとServiceAccountの紐付け (Secret経由)

2. **クライアント証明書認証** (mTLS)
   - TLSクライアント証明書のCommon Name → username
   - Organization → groups
   - 既存のTLS設定基盤を活用

3. **(将来) 静的トークンファイル認証**

### ミドルウェアの動作

1. リクエストヘッダー/TLS情報から認証情報を抽出
2. 認証成功時: `UserInfo` をリクエストextensionsに挿入
3. 認証失敗時: 401 Unauthorized を返す
4. 認証情報なし: anonymous userとして扱う (設定可能)

## 作業内容

1. `tugboat-apiserver/src/auth/` モジュールを作成:
   - `mod.rs` - モジュール定義
   - `user_info.rs` - UserInfo構造体
   - `authenticator.rs` - 認証トレイトと実装
   - `middleware.rs` - actix-webミドルウェア

2. `tugboat-apiserver/src/lib.rs` の `App::new()` にミドルウェアを追加

3. 設定ファイルに認証関連設定を追加:
   ```toml
   [authentication]
   anonymous_enabled = true
   # token_auth_file = "/etc/tugboat/known_tokens.csv"
   ```

## 確認

```bash
cargo build --package tugboat-apiserver
cargo test --package tugboat-apiserver
```

## 注意

- この段階ではまだ認可 (Authorization) は行わない。認証のみ
- 既存のエンドポイントが壊れないよう、anonymous accessをデフォルトで有効にする
- `StatusResponse::unauthorized()` がコメントアウトされているので有効化する

## 参考

- actix-webミドルウェア: `actix_web::middleware` / `Transform` トレイト
- 既存TLS設定: `tugboat-apiserver/src/lib.rs`
