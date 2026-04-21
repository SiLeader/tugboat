# Task 6: migrate / migrate-cancel / migration-status サブコマンド

## 目的

Cloud Hypervisor REST API経由でライブマイグレーションの開始、キャンセル、状態取得を実装する。

## 依存タスク

- Task 1（スキャフォールド）
- Task 2（APIクライアント）

## 実装内容

### 1. 作成するファイル

| ファイル | 内容 |
|---------|------|
| `src/cmd/migrate/mod.rs` | マイグレーション開始 |
| `src/cmd/migrate_cancel/mod.rs` | マイグレーションキャンセル |
| `src/cmd/migration_status/mod.rs` | マイグレーション状態取得 |

### 2. migrate サブコマンド

`VmMigrateRequest` を読み取り、`PUT /api/v1/vm.send-migration` を呼ぶ:

```json
{
    "destination_url": "tcp:{destination_address}:{destination_port}",
    "local": false
}
```

実装フロー:
1. `VmMigrateRequest` をstdinからJSON読み取り
2. `validate_safe_id` でIDをバリデーション
3. APIソケットに接続
4. `PUT /api/v1/vm.send-migration` を実行

**注意**: Cloud HypervisorのマイグレーションAPIはQEMUと比較して以下の制限がある:
- 帯域幅制限（`max_bandwidth_bytes_per_sec`）はサポートされない → 無視（ログで警告）
- ダウンタイム制限（`downtime_limit_ms`）はサポートされない → 無視（ログで警告）
- XBZRLE（`xbzrle_cache_size_bytes`）はサポートされない → 無視（ログで警告）
- Postcopy（`postcopy_enabled`）はサポートされない → 無視（ログで警告）

### 3. migrate-cancel サブコマンド

Cloud Hypervisorにはマイグレーションキャンセル専用のAPIがない。

実装方針: `Error::ActionFailed` を返す:
```
Cloud Hypervisor does not support migration cancellation
```

### 4. migration-status サブコマンド

`GET /api/v1/vm.info` から状態を推測して `VmMigrationStatusResponse` を構築:

Cloud Hypervisorは詳細なマイグレーション進捗情報を公開しないため、VMの状態から推測する:

| vm.info state | VmMigrationPhase | 判断根拠 |
|--------------|-----------------|---------|
| `"Running"` | `None` or `Completed` | マイグレーション前 or 完了後 |
| `"Paused"` | `Active` | マイグレーション中の可能性 |
| `"Shutdown"` | `Completed` | 送信側がシャットダウン |

**注意**: `bytes_transferred`, `bytes_remaining`, `ram_dirty_rate_mbps` は全て `None` になる（Cloud Hypervisorが公開していないため）。

### 5. 参照ファイル

- `tugboat-qemu-runtime/src/cmd/migrate/mod.rs` (138行): QMP版マイグレーション実装
- `tugboat-qemu-runtime/src/cmd/migrate_cancel/mod.rs` (38行): QMP版キャンセル実装
- `tugboat-qemu-runtime/src/cmd/migration_status/mod.rs` (78行): QMP版状態取得実装
- `tugboat-vm-runtime-interface/src/migrate.rs` (73行): マイグレーション関連の型定義

## 受け入れ条件

- [ ] `migrate` が `PUT /api/v1/vm.send-migration` を正しいURLで呼ぶ
- [ ] サポートされないパラメータ（bandwidth, downtime, xbzrle, postcopy）使用時にログ警告が出る
- [ ] `migrate-cancel` が `Error::ActionFailed` を返す
- [ ] `migration-status` が `VmMigrationStatusResponse` をJSON形式でstdoutに出力する
- [ ] 転送統計フィールドが `None` になる
- [ ] `cargo test` がパスする

## テスト・確認方法

ユニットテスト（モックHTTPサーバー使用）:

```rust
#[tokio::test]
async fn test_migrate_sends_correct_url() {
    // send-migration のリクエストボディに正しいdestination_urlが含まれることを検証
}

#[tokio::test]
async fn test_migrate_cancel_returns_error() {
    // migrate-cancel が ActionFailed エラーを返すことを検証
}

#[tokio::test]
async fn test_migration_status_running() {
    // vm.infoがRunning状態のときのレスポンスを検証
}
```

```bash
cargo test --package tugboat-cloud-hypervisor-runtime
cargo clippy --package tugboat-cloud-hypervisor-runtime
```
