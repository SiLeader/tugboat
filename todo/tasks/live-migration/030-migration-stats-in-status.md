# マイグレーション統計情報を ShipMigrationStatus に追加

## 概要

QEMU の `query-migrate` QMP コマンドは転送バイト数・残余バイト数・RAM ダーティページレート等の
詳細な統計情報を返す。現状の `VmMigrationStatusResponse` はフェーズとメッセージのみを保持しており、
進捗情報がユーザに見えない。

マイグレーション中の可視性を高めるために、統計情報を `ShipMigrationStatus` および
`VmMigrationStatusResponse` に追加する。

## 対象ファイル

- `tugboat-vm-runtime-interface/src/migrate.rs`
- `tugboat-runtime/src/cmd/migration_status/mod.rs`
- `tugboat-resources/proto/core/v1/ship.proto`
- `tugboat-resources/build.rs` (proto 再コンパイル)
- `tugboat-agent/src/reconciler/ops/migration.rs` (status 更新時に stats を埋める)

## 実装内容

### 1. `VmMigrationStatusResponse` に統計フィールドを追加（`migrate.rs`）

```rust
pub struct VmMigrationStatusResponse {
    pub phase: VmMigrationPhase,
    pub message: String,
    // 以下を追加
    pub bytes_transferred: Option<u64>,
    pub bytes_remaining: Option<u64>,
    pub ram_dirty_rate_mbps: Option<f64>,
}
```

### 2. `migration_status/mod.rs` で QEMU 統計を読み取る

`query_migrate` レスポンスの `ram` フィールドから:
- `transferred` → `bytes_transferred`
- `remaining` → `bytes_remaining`
- `dirty_pages_rate` → `ram_dirty_rate_mbps`

### 3. `ship.proto` の `ShipMigrationStatus` に stats フィールドを追加

```protobuf
optional uint64 bytes_transferred = 8;
optional uint64 bytes_remaining = 9;
```

### 4. エージェントの `PHASE_MIGRATING` 状態更新時に stats を転記

`update_migration_status` 呼び出し時に `VmMigrationStatusResponse` から
stats を `ShipMigrationStatus` に詰める。

## 完了条件

- `cargo build --release` が通ること
- マイグレーション中の Ship ステータスに転送バイト数・残余バイト数が反映される
- stats が取れない場合（マイグレーション未開始等）は `None` / 省略となること
