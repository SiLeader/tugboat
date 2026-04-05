# テスト基盤の構築

## 概要

E2E 結合テストを実行するための基盤を整備する。
API サーバーと etcd を起動し、テストクライアント経由で操作できる環境を構築する。

## 対象ファイル

- `tests/integration/` ディレクトリを新規作成
- `tests/integration/helpers/mod.rs` — 共通ヘルパー
- `tests/integration/helpers/setup.rs` — テスト環境セットアップ
- `Cargo.toml`（ワークスペースへのテストクレート追加）

## 実装内容

### 1. テスト用クレート構成

ワークスペース直下に結合テスト用クレートまたは `tests/` ディレクトリを用意する。
`tugboat-client` と `tugboat-resources` を依存に追加する。

### 2. テスト環境セットアップヘルパー

以下の機能を持つヘルパーモジュールを作成する:

- **etcd 起動/停止**: `docker run` または既存 etcd プロセスへの接続
- **API サーバー起動**: `tugboat-apiserver` バイナリをバックグラウンドで起動
- **ヘルスチェック待機**: `/healthz` エンドポイントが 200 を返すまでリトライ
- **クリーンアップ**: テスト終了時にプロセスを停止し etcd データを削除
- **TugboatClient 生成**: テスト用のクライアントインスタンスを返す

### 3. テスト実行制御

- 外部依存（etcd, QEMU 等）が不要なテストは通常の `cargo test` で実行
- etcd/API サーバーが必要なテストは `#[ignore]` を付与するか、feature flag `integration` で制御
- 環境変数 `TUGBOAT_TEST_APISERVER_URL` が設定済みの場合は既存環境を使用

### 4. docker-compose.test.yml（オプション）

テスト用の最小構成 docker-compose を用意する:

```yaml
services:
  etcd:
    image: quay.io/coreos/etcd:v3.6.7
    ports:
      - "2379:2379"
  apiserver:
    build: .
    depends_on:
      etcd:
        condition: service_healthy
    ports:
      - "8080:8080"
```

## 完了条件

- テストヘルパーを使って API サーバーに接続し `/healthz` を呼び出すテストが通ること
- `cargo test --package <test-crate>` でヘルスチェックテストが成功すること（etcd 起動済み環境）
- CI 環境で etcd が無い場合はスキップされること
