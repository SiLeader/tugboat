# Post-copy マイグレーションのサポート

## 概要

現状のマイグレーションは pre-copy 方式のみ（QEMU デフォルト）。
メモリ使用量が大きい Ship では収束しにくく、`MIGRATION_ACTIVE_TIMEOUT_SECS` (30分) を
超えてタイムアウトする可能性がある。

QEMU の post-copy モードを有効化するオプションを `MigrationSpec` に追加し、
大メモリ Ship の移行ダウンタイムを短縮できるようにする。

Post-copy では pre-copy 段階が終わった後に VM をターゲットに切り替え、
残りのページをオンデマンドで転送する。ダウンタイムは短縮されるが、
転送中はネットワーク遅延がページフォルト遅延に影響するトレードオフがある。

## 対象ファイル

- `tugboat-resources/proto/core/v1/ship_class.proto`（`MigrationSpec` への `postcopy` フィールド追加）
- `tugboat-vm-runtime-interface/src/migrate.rs`（`VmMigrateRequest` にフラグ追加）
- `tugboat-runtime/src/cmd/migrate/mod.rs`（QEMU の `postcopy-ram` capability 設定）
- `tugboat-agent/src/reconciler/ops/migration.rs`（`resolve_migration_params` でフィールド転記）

## 実装内容

### 1. `MigrationSpec` にフィールド追加（proto）

```protobuf
message MigrationSpec {
  optional uint64 max_bandwidth_bytes_per_sec = 1;
  optional uint64 downtime_limit_ms = 2;
  optional uint64 xbzrle_cache_size_bytes = 3;
  optional bool postcopy_enabled = 4;  // 追加
}
```

### 2. `VmMigrateRequest` にフラグ追加

```rust
pub postcopy_enabled: bool,
```

### 3. `migrate/mod.rs` で postcopy capability を条件付き有効化

`req.postcopy_enabled == true` の場合に `MigrationCapability::postcopy_ram` を
`migrate_set_capabilities` に追加する。

### 4. `VmMigrationParams` に `postcopy_enabled: bool` を追加して agent から渡す

### 注意事項

- post-copy 中のページ転送エラーは VM クラッシュにつながるため、
  ネットワークの信頼性が高い環境でのみ使用を推奨するドキュメントコメントを追加すること
- post-copy 有効時は `MigrationStatus::postcopy_active` 等が返るが、
  `migration_status/mod.rs` で `VmMigrationPhase::Active` にマップ済みのため変更不要

## 完了条件

- `postcopy_enabled: true` を設定した ShipClass で migrate を呼ぶと postcopy capability が有効になる
- `postcopy_enabled: false`（デフォルト）では従来どおり pre-copy のみ
- `cargo build --release` が通ること
