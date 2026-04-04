# ライブマイグレーションの結合テスト（E2E）

## 概要

現状のマイグレーション関連テストはすべて `FakeContext` を使ったユニットテストであり、
実際の QEMU プロセスや HTTP API を通じた動作は検証されていない。

エンドツーエンドテストを追加し、実際のマイグレーションフローを検証する。

## 対象ファイル

- `tugboat-agent/tests/` または `tests/` 以下に新規ファイルを作成
- QEMU が利用可能な環境でのみ実行するよう `#[ignore]` または feature flag を使用

## テストシナリオ

### シナリオ 1: 正常系マイグレーション

1. source ノード上で Ship（RWX PVC 付き）を起動
2. `spec.targetNodeName` を target ノードに設定
3. 移行フェーズが `Pending` → `Ready` → `Migrating` → `Completed` と遷移することを確認
4. `spec.nodeName` が target ノードに更新されること
5. `spec.targetNodeName` が `null` になること
6. source ノードに QEMU プロセスが残っていないこと

### シナリオ 2: Preflight 拒否（RWO ストレージ）

1. RWO PVC のみを持つ Ship で `spec.targetNodeName` を設定
2. `status.migration.phase` が `Failed` に遷移し、preflight rejection メッセージが含まれること
3. `spec.nodeName` が変わっていないこと（source のまま）

### シナリオ 3: タイムアウト

1. target ノードが QEMU receiver を起動しない状態でマイグレーションを開始
2. `MIGRATION_PENDING_TIMEOUT_SECS`（120秒）後に `Failed` に遷移すること

## 完了条件

- 少なくとも正常系とストレージ拒否ケースのテストが実装されていること
- CI 環境での実行設定（QEMU 非インストール環境ではスキップ）
- `cargo test` が通ること（QEMU あり環境では追加テストも合格）
