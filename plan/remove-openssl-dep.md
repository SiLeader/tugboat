# Rust プログラムから OpenSSL 依存を完全に削除する実装計画

## 結論

実現可能です。

ただし、API サーバーの TLS 実装だけを `rustls` に置き換えても完了しません。現在の Rust 依存グラフには、次の 2 系統から OpenSSL が入っています。

1. `tugboat-apiserver` が直接利用する `openssl` と、Actix Web の `openssl` feature
2. `tugboat-vm-image` が利用する `oci-distribution` の既定 `native-tls` feature

さらに、API サーバー内では TLS 以外にも JWT 署名・検証、OIDC JWKS 検証、クライアント証明書の subject 解析、テスト用証明書生成に OpenSSL API を使用しています。以下の計画ではこれらをすべて置換し、Linux を含む全ターゲットで `openssl-sys` と `libssl` へのリンクが発生しない状態を完了条件とします。

インストーラーの `installer/systemd/setup-pki.sh`、`installer/systemd/setup-etcd-pki.sh` などが外部コマンドとして呼び出す `openssl` CLI は「Rust プログラムの依存」ではないため本計画の対象外です。ホストから `openssl` パッケージ自体も削除する場合は、PKI 生成ツールの置換を別タスクとして実施する必要があります。

## 現状

### 直接依存

- ルート `Cargo.toml`
  - workspace dependency として `openssl` を定義
- `tugboat-apiserver/Cargo.toml`
  - `actix-web` の `openssl` feature を有効化
  - `openssl.workspace = true`
  - `actix-tls` の default features により OpenSSL acceptor も有効化
- `tests/integration/Cargo.toml`
  - 証明書・秘密鍵生成用に `openssl.workspace = true`

### API サーバー内の OpenSSL 利用箇所

- `tugboat-apiserver/src/lib.rs`
  - `SslAcceptor` による HTTPS/mTLS 設定
  - `bind_openssl` / `listen_openssl`
  - OpenSSL TLS stream から peer certificate を取得
- `tugboat-apiserver/src/auth/middleware.rs`
  - `X509Ref` と `Nid` によるクライアント証明書の CN/O 抽出
- `tugboat-apiserver/src/auth/service_account_jwt.rs`
  - RSA/Ed25519 PEM の読み込み
  - RS256/EdDSA の署名・検証
  - 公開鍵導出、アルゴリズム判定、JWKS 生成
- `tugboat-apiserver/src/auth/oidc.rs`
  - RSA JWK の復元
  - RS256/EdDSA の署名検証
- 上記各モジュールの unit test
  - RSA/Ed25519 鍵や X.509 証明書の生成
- `tests/integration/helpers/setup.rs`
  - CA、server/client certificate、ServiceAccount 署名鍵の生成

### 全ターゲットでのみ現れる追加経路

`oci-distribution` 0.11 の default features は `native-tls` を含みます。Linux では次の経路で OpenSSL に到達します。

```text
tugboat-vm-image
  -> oci-distribution(default/native-tls)
  -> reqwest 0.12
  -> native-tls
  -> openssl / openssl-sys
```

macOS 上の通常の `cargo tree -i openssl` だけではこの Linux 固有経路を見落とすため、完了確認では `--target all` または CI 対象 Linux triple を必ず使用します。

## 目標構成

| 用途 | 置換先 |
|---|---|
| Actix HTTPS/mTLS | `rustls` 0.23、Actix の `rustls-0_23` feature |
| PEM 証明書・秘密鍵読み込み | `rustls-pemfile`、`rustls-pki-types` |
| TLS 暗号 provider | `ring` provider を明示選択 |
| クライアント証明書 subject 解析 | `x509-parser` |
| OIDC RS256/EdDSA 検証 | `ring::signature` |
| ServiceAccount の RSA 鍵処理と RS256 | RustCrypto `rsa`、`signature`、`sha2` |
| ServiceAccount の Ed25519 鍵処理と EdDSA | `ed25519-dalek`、`pkcs8` |
| テスト用 X.509 証明書生成 | `rcgen` |
| OCI registry HTTPS | `oci-distribution` の `rustls-tls` feature |

`ring` は OpenSSL をリンクしません。ただし一部に C/assembly 実装を含むため、「暗号実装も 100% pure Rust」という別要件がある場合は対象外です。本計画の要件である OpenSSL/libssl 非依存は満たします。

## 実装方針

### 1. Cargo feature と依存関係を先に切り替える

対象:

- `Cargo.toml`
- `tugboat-apiserver/Cargo.toml`
- `tests/integration/Cargo.toml`
- `Cargo.lock`

変更:

1. workspace の `openssl` dependency を削除する。
2. workspace dependencies に `rustls`、`rustls-pemfile`、`ring`、`x509-parser`、`rsa`、`signature`、`ed25519-dalek`、`pkcs8`、`rcgen` を追加する。
3. `tugboat-apiserver` の `actix-web` feature を `openssl` から `rustls-0_23` に変更する。
4. `actix-tls` は default features を無効化し、peer certificate 取得に必要な `accept` と `rustls-0_23` のみを有効化する。
5. `tugboat-apiserver` から `openssl` を削除し、本番コード用の rustls/暗号/X.509 dependencies を追加する。
6. `tests/integration` から `openssl` を削除し、テスト用に `rcgen` と RSA/Ed25519 鍵生成に必要な RustCrypto crates を追加する。
7. `oci-distribution` を次の方針に変更する。

   ```toml
   oci-distribution = {
       version = "0.11.0",
       default-features = false,
       features = ["rustls-tls"],
   }
   ```

8. `cargo update` または通常の build で `Cargo.lock` を再生成する。

注意:

- workspace の `reqwest` 0.13 は既に rustls を既定 TLS backend として使用しているため、直接の native-tls 切り替えは不要。
- `rustls` 0.23 は feature 組み合わせにより `aws-lc-rs` と `ring` が同時に有効になり得る。API サーバーの `ServerConfig` 作成時には `builder()` の暗黙選択に依存せず、`rustls::crypto::ring::default_provider()` を `builder_with_provider` に明示的に渡す。
- `openssl-probe` は `rustls-native-certs` が Unix 上の CA 証明書探索に使う、libssl をリンクしない小さな crate である。OpenSSL FFI 依存の削除条件には含めない。crate 名の文字列も含めて排除する要件がある場合は、OS trust store を使う `reqwest` の既定構成を見直し、WebPKI roots または明示的な root store に統一する追加作業が必要。

### 2. API サーバー TLS を rustls 0.23 に移行する

対象:

- `tugboat-apiserver/src/lib.rs`

実装:

1. `build_tls_acceptor` を `build_tls_server_config` に置き換え、`rustls::ServerConfig` を返す。
2. `rustls-pemfile` で `cert_file` から証明書 chain を全件読み込む。
3. `key_file` から PKCS#8、PKCS#1 RSA、SEC1 EC の秘密鍵を読み込めるようにする。
4. 証明書が空、秘密鍵がない、秘密鍵が複数、証明書と鍵が不一致などの設定エラーを起動時に明示的に返す。
5. 通常 TLS は `with_no_client_auth().with_single_cert(...)` で構築する。
6. mTLS は `client_cert_file` の全証明書を `RootCertStore` に登録し、`WebPkiClientVerifier` を使ってクライアント証明書を必須にする。
7. OpenSSL の `PEER | FAIL_IF_NO_PEER_CERT` と同じく、証明書なしの接続を TLS handshake で拒否する。Bearer token と mTLS の現行排他 semantics は変更しない。
8. `bind_openssl` / `listen_openssl` を `bind_rustls_0_23` / `listen_rustls_0_23` に変更する。
9. ALPN に `h2` と `http/1.1` を設定し、OpenSSL backend からの切り替えで HTTP/2 を失わないようにする。

エラー文は現行の「証明書ファイル読み込み」「秘密鍵読み込み」「client CA 読み込み」「TLS 設定失敗」の区別を維持し、PEM parse failure の原因が分かる粒度にする。

### 3. rustls 接続からクライアント証明書情報を取得する

対象:

- `tugboat-apiserver/src/lib.rs`
- `tugboat-apiserver/src/auth/middleware.rs`

実装:

1. `on_connect` 内の downcast 対象を `actix_tls::accept::rustls_0_23::TlsStream<...>` に変更する。
2. rustls `ServerConnection::peer_certificates()` から leaf certificate の DER を取得する。
3. DER を `x509-parser` で解析し、subject の最初の Common Name と全 Organization を `ClientCertificateInfo` にコピーする。
4. 解析失敗を認証情報なしとして黙って扱わない。TLS 検証済み証明書の解析に失敗した場合は警告を記録し、そのリクエストを証明書認証不可にする。
5. CN なし、O が複数、UTF8String 以外の DirectoryString、証明書 chain の leaf 選択を unit test で固定する。

認証・認可層には `ClientCertificateInfo` だけを渡し続け、OpenSSL 型や rustls 型を middleware の後段へ漏らさない。

### 4. ServiceAccount JWT を RustCrypto 実装へ移行する

対象:

- `tugboat-apiserver/src/auth/service_account_jwt.rs`

内部表現:

```text
SigningKeyMaterial
  - Rsa(RsaPrivateKey)
  - Ed25519(ed25519_dalek::SigningKey)

VerificationKeyMaterial
  - Rsa(RsaPublicKey)
  - Ed25519(ed25519_dalek::VerifyingKey)
```

実装:

1. RS256:
   - PKCS#8 と PKCS#1 の PEM/DER 秘密鍵を受け付ける。
   - SPKI `PUBLIC KEY`、PKCS#1 `RSA PUBLIC KEY`、RSA private key から verification key を読み込めるようにする。
   - `rsa::pkcs1v15::SigningKey<Sha256>` / `VerifyingKey<Sha256>` と `signature` traits を使用する。
   - PSS ではなく JWT RS256 が要求する PKCS#1 v1.5 + SHA-256 を維持する。
2. EdDSA:
   - Ed25519 PKCS#8 private key と SPKI public key を受け付ける。
   - `ed25519-dalek` の `SigningKey` / `VerifyingKey` を使用する。
3. 現行の `signing_algorithm` と鍵種別の不一致エラーを維持する。
4. `additional_verification_keys` が公開鍵だけでなく秘密鍵 PEM も受理する現行互換性を維持する。
5. JWKS:
   - RSA は `PublicKeyParts::n()` / `e()` を unsigned big-endian で base64url 化する。
   - Ed25519 は 32-byte public key を `x` に設定する。
   - `kty`、`alg`、`use`、`crv` の現行 JSON shape を変更しない。
6. `kid`:
   - 現行は OpenSSL が生成した public-key PEM の SHA-256 先頭 12 bytesである。
   - RustCrypto で SPKI public key を同じ `PUBLIC KEY` PEM 形式・LF・64 文字折り返しで canonicalize し、既存 fixture に対して同一 `kid` になることを golden test で保証する。
   - 同値を保証できない場合は暗黙に変更せず、移行リリース前に `signing_key_id` の明示設定を必須化し、旧 `kid` を `additional_verification_keys` に残すローテーション手順を文書化する。
7. `ServiceAccountTokenIssuer` と `VerificationKey` の外部挙動、claim validation、OpenID configuration endpoint は変更しない。

### 5. OIDC JWKS 検証を ring に移行する

対象:

- `tugboat-apiserver/src/auth/oidc.rs`

実装:

1. `JwksEntry` は OpenSSL `PKey` ではなく、アルゴリズム別の所有データを保持する。
   - RS256: JWK の `n` と `e`
   - EdDSA: 32-byte Ed25519 public key
2. RS256 は `ring::signature::RsaPublicKeyComponents` と `RSA_PKCS1_2048_8192_SHA256` で検証する。
3. EdDSA は `ring::signature::UnparsedPublicKey` と `ED25519` で検証する。
4. RSA modulus:
   - 最低 2048 bit を要求する。
   - 空の `n`/`e`、leading-zero の不正表現、範囲外 exponent を明示的な JWKS/claim error にする。
5. Ed25519:
   - `crv == "Ed25519"` を維持する。
   - `x` が厳密に 32 bytes であることを parse 時に検証する。
6. `JwksEntry::clone_for_use` は byte buffer/固定長配列の clone に変更し、OpenSSL refcount に関するコメントを削除する。
7. issuer、audience、有効期限、required claims、unknown `kid` refresh の既存ロジックは変更しない。

テスト用 JWT の署名には本番側と独立した RustCrypto key/signing API を使い、検証側の実装ミスを同じ helper で相殺しないようにする。

### 6. OpenSSL を使うテスト fixture 生成を置換する

対象:

- `tugboat-apiserver/src/lib.rs` の TLS unit test
- `tugboat-apiserver/src/auth/middleware.rs` の X.509 unit test
- `tugboat-apiserver/src/auth/service_account_jwt.rs` の unit test
- `tugboat-apiserver/src/auth/oidc.rs` の unit test
- `tests/integration/helpers/setup.rs`

実装:

1. TLS 証明書:
   - `rcgen` でテスト CA を生成する。
   - server certificate に `127.0.0.1` の IP SAN と serverAuth EKU を設定する。
   - mTLS client certificate に CN `masters-user`、O `system:masters`、clientAuth EKU を設定する。
   - private key は rustls/reqwest が読める PKCS#8 PEM で出力する。
2. ServiceAccount RS256:
   - RustCrypto `rsa` で 2048-bit key を生成し PKCS#8 PEM へ出力する。
3. ServiceAccount/OIDC EdDSA:
   - `ed25519-dalek` で key を生成し PKCS#8/SPKI へ出力する。
4. OpenSSL で生成済みの固定 fixture を必要最小限追加し、旧 PEM 入力互換性と `kid` 互換性だけを検証する。fixture の生成をテスト実行時に OpenSSL へ依存させない。
5. 一時ファイル cleanup と既存の HTTPS/mTLS test flow は維持する。

### 7. OCI registry client を rustls に固定する

対象:

- `Cargo.toml`
- 必要に応じて `tugboat-vm-image` の registry tests

実装:

1. `oci-distribution` の default features を無効化し、`rustls-tls` のみを明示する。
2. `VmImageRegistry::get_client` の HTTP/HTTPS 選択動作が変わらないことを確認する。
3. HTTPS registry に対する pull/push、認証、redirect の既存 test があれば実行する。
4. private registry/custom CA が将来必要な場合は、native-tls を戻さず rustls root store を注入できる API を追加する。

### 8. 再導入防止とドキュメント更新

対象:

- `deny.toml`
- 必要に応じて CI workflow
- TLS/ServiceAccount/OIDC の関連ドキュメント

実装:

1. `cargo-deny` の bans に少なくとも次を追加する。
   - `openssl`
   - `openssl-sys`
   - `tokio-openssl`
   - `native-tls`
   - `tokio-native-tls`
   - `hyper-tls`
2. `deny.toml` の license allow list にある `OpenSSL` はライセンス識別子であり、OpenSSL crate の許可ではない。依存削除とは独立に、`cargo deny check licenses` の結果を確認して不要なら削除する。
3. TLS 設定ファイルの schema と PEM ファイル要件は変えない。受理形式を狭める場合だけ `docs` と sample config を更新する。
4. インストーラー文書の `openssl` パッケージ記載は本計画では削除しない。外部 PKI 生成スクリプトが引き続き必要とするため。

## 実装順序

1. `oci-distribution` を rustls feature に変更し、Linux 固有の native-tls/OpenSSL 経路を先に除去する。
2. Actix HTTPS/mTLS を rustls へ変更する。
3. client certificate の DER subject 解析を `x509-parser` へ変更する。
4. OIDC 検証を `ring` へ変更する。
5. ServiceAccount JWT 鍵処理・署名・検証を RustCrypto へ変更する。
6. unit/integration test の鍵・証明書生成を OpenSSL から置換する。
7. manifests と `Cargo.lock` から OpenSSL/native-tls crates を除去する。
8. `cargo-deny` ban と CI の依存グラフ検査を追加する。
9. 全検証を通し、既存 TLS/JWT/OIDC/mTLS の外部挙動が変わっていないことを確認する。

各段階はコンパイル可能な単位で行う。ただし `openssl` dependency の最終削除は、production code と test code の全 OpenSSL import を除去した段階でまとめて行う。

## 必須テスト

### TLS

- server certificate chain と PKCS#8 private key で起動できる。
- PKCS#1 RSA private key も既存互換として読める。
- 不正 PEM、空 certificate chain、鍵なし、証明書と鍵の不一致で起動に失敗する。
- server-only TLS で HTTPS health check が成功する。
- mTLS で trusted client certificate は成功する。
- mTLS で client certificate なし、未知 CA、期限切れ証明書は handshake で失敗する。
- mTLS 証明書の CN/O が既存どおり `UserInfo::x509` に反映される。
- HTTP/1.1 と HTTP/2 の両方を維持する。

### ServiceAccount JWT

- RS256 と EdDSA の発行・自己検証。
- 旧 OpenSSL 生成 PKCS#8/PKCS#1/SPKI PEM fixture の読み込み。
- 署名 algorithm と鍵種別の不一致拒否。
- additional verification key による key rotation。
- 既存 `kid` の互換性。
- JWKS の RSA `n`/`e` と Ed25519 `x` が既存形式と一致する。
- 改ざん、誤 audience、期限切れ、未来の `nbf`/`iat` を拒否する。

### OIDC

- RS256 と EdDSA の正常検証。
- 不正署名、未知 `kid`、algorithm mismatch を拒否する。
- 2048 bit 未満の RSA key、異常な exponent、不正長 Ed25519 key を拒否する。
- JWKS refresh、issuer/audience/required claims の既存 test を維持する。

### OCI

- HTTPS registry に対する pull/push。
- 認証付き registry。
- 明示的 insecure HTTP registry の既存挙動。

## 完了条件

以下をすべて満たした時点で完了とする。

1. Rust source と Cargo manifests に OpenSSL API/feature がない。

   ```bash
   rg 'openssl::|features\s*=\s*\[[^]]*"openssl"|openssl\.workspace' \
     --glob '*.rs' --glob 'Cargo.toml'
   ```

   結果が 0 件であること。

2. Linux を含む有効依存グラフに OpenSSL/native-tls backend がない。

   ```bash
   cargo tree --workspace --all-features \
     --target x86_64-unknown-linux-musl -e all
   ```

   出力に次の crates がないこと。

   - `openssl`
   - `openssl-sys`
   - `tokio-openssl`
   - `native-tls`
   - `tokio-native-tls`
   - `hyper-tls`

3. Rust binary が OpenSSL を動的リンクしない。

   Linux:

   ```bash
   cargo build --release --workspace
   find target/release -maxdepth 1 -type f -perm -111 -exec sh -c \
     'ldd "$1" 2>/dev/null | grep -E "libssl|libcrypto" && exit 1 || exit 0' _ {} \;
   ```

   macOS:

   ```bash
   find target/release -maxdepth 1 -type f -perm -111 -exec sh -c \
     'otool -L "$1" 2>/dev/null | grep -E "libssl|libcrypto" && exit 1 || exit 0' _ {} \;
   ```

4. 必須 gate が成功する。

   ```bash
   cargo fmt --check
   cargo clippy --workspace --all-targets --all-features -- -D warnings
   cargo test --workspace --all-targets
   cargo deny check
   ```

5. TLS、mTLS、ServiceAccount RS256/EdDSA、OIDC RS256/EdDSA、OCI HTTPS の各経路が integration test で成功する。

## 主なリスクと対策

| リスク | 対策 |
|---|---|
| OpenSSL と rustls で受理する PEM/key 形式が異なる | PKCS#8、PKCS#1、SEC1、SPKI の互換 test を追加し、非対応形式は起動時に明示する |
| rustls provider が複数有効で暗黙選択に失敗する | `ring` provider をコードで明示し、暗黙の process-global default に依存しない |
| mTLS の client certificate 必須条件が弱くなる | `WebPkiClientVerifier` で証明書を必須化し、証明書なし handshake failure を integration test する |
| X.509 subject の文字列変換差で認証 identity が変わる | CN/O の fixture test を追加し、変換不能値を黙って採用しない |
| ServiceAccount の自動 `kid` が変わり既存 token が無効になる | OpenSSL 生成 fixture との golden test、または明示 `signing_key_id` + verification key rotation を必須化する |
| OIDC RSA JWK の不正値を ring に渡すまで検出できない | modulus/exponent の構造・最小強度を parse 時に検証する |
| macOS のみの確認で Linux native-tls 経路を見落とす | Linux target の dependency graph を CI gate にする |
| OpenSSL CLI 依存も削除済みと誤認する | Rust dependency と installer PKI tool dependency のスコープを文書で分離する |
