# Task 1: クレート構造のスキャフォールドとCLIディスパッチ

## 目的

コンパイル可能でclippy-cleanなクレートを作成する。clap CLIディスパッチ、エラー型、ハイパーバイザ非依存モジュール（QEMU runtimeからコピー）を含む。

## 依存タスク

なし（最初のタスク）

## 実装内容

### 1. Cargo.toml の依存関係設定

`tugboat-cloud-hypervisor-runtime/Cargo.toml` に以下を追加:

```toml
[dependencies]
tugboat-resources.workspace = true
tugboat-vm-runtime-interface.workspace = true
clap = { workspace = true, features = ["derive"] }
tokio = { workspace = true, features = ["full"] }
futures.workspace = true
async-trait.workspace = true
thiserror.workspace = true
serde = { workspace = true, features = ["derive"] }
toml.workspace = true
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber = { workspace = true, features = ["env-filter"] }
nix = { workspace = true, features = ["sched", "user", "signal", "fs", "mount"] }
```

### 2. 作成するファイル

| ファイル | 元ファイル | 変更点 |
|---------|-----------|--------|
| `src/main.rs` | `tugboat-qemu-runtime/src/main.rs` | そのままコピー |
| `src/lib.rs` | `tugboat-qemu-runtime/src/lib.rs` | `QemuVmConfig` → `CloudHypervisorVmConfig`, `Qmp` エラー → `Api` エラー, Config struct 変更 |
| `src/config.rs` | `tugboat-qemu-runtime/src/config.rs` | そのままコピー |
| `src/validate.rs` | `tugboat-qemu-runtime/src/validate.rs` | `validate_safe_id` のみコピー（`validate_qemu_option_value` は不要） |
| `src/pre/mod.rs` | `tugboat-qemu-runtime/src/pre/mod.rs` | そのままコピー |
| `src/utils/mod.rs` | `tugboat-qemu-runtime/src/utils/mod.rs` | そのままコピー |
| `src/cmd/mod.rs` | 新規 | 各サブコマンドモジュールの宣言 |
| `src/cmd/create/mod.rs` | 新規 | `CreateArgs` struct のみ定義、`todo!()` |
| `src/cmd/start/mod.rs` | 新規 | `StartArgs` struct のみ定義、`todo!()` |
| `src/cmd/run/mod.rs` | 新規 | `StartArgs` struct のみ定義、`todo!()` |
| `src/cmd/stop/mod.rs` | 新規 | `StopArgs` struct のみ定義、`todo!()` |
| `src/cmd/status/mod.rs` | 新規 | `StatusArgs` struct のみ定義、`todo!()` |
| `src/cmd/hotplug/mod.rs` | 新規 | `HotplugArgs` struct のみ定義、`todo!()` |
| `src/cmd/migrate/mod.rs` | 新規 | `MigrateArgs` struct のみ定義、`todo!()` |
| `src/cmd/migrate_cancel/mod.rs` | 新規 | `MigrateCancelArgs` struct のみ定義、`todo!()` |
| `src/cmd/migration_status/mod.rs` | 新規 | `MigrationStatusArgs` struct のみ定義、`todo!()` |
| `src/execute/mod.rs` | 新規 | 空モジュール |

### 3. CloudHypervisorVmConfig 定義

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct CloudHypervisorVmConfig {
    pub executable: String,           // cloud-hypervisor バイナリパス
    pub disk_image_location: String,  // ディスクイメージとAPIソケットの保存先
    pub boot: CloudHypervisorBootConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloudHypervisorBootConfig {
    pub kernel: Option<String>,     // vmlinux パス (直接カーネルブート)
    pub initramfs: Option<String>,  // initramfs パス (オプション)
    pub firmware: Option<String>,   // UEFI ファームウェアパス (OVMF)
}
```

### 4. Error 型

```rust
#[derive(Debug, Error)]
pub enum Error {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON Error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML Error: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("System call Error: {0}")]
    Syscall(#[from] Errno),
    #[error("Failed to setup network: {0}")]
    NetworkSetupFailed(String),
    #[error("Cloud Hypervisor API error: {0}")]
    Api(String),
    #[error("Action failed: {0}")]
    ActionFailed(String),
    #[error("Validation error: {0}")]
    Validation(String),
}
```

### 5. 参照ファイル

- `tugboat-qemu-runtime/src/lib.rs` (186行): CLI構造とエラー型のリファレンス
- `tugboat-qemu-runtime/src/config.rs` (28行): コピー元
- `tugboat-qemu-runtime/src/validate.rs` (109行): `validate_safe_id` のコピー元
- `tugboat-qemu-runtime/src/pre/mod.rs` (125行): コピー元
- `tugboat-qemu-runtime/src/utils/mod.rs` (73行): コピー元
- `tugboat-qemu-runtime/src/main.rs` (24行): コピー元

## 受け入れ条件

- [ ] `cargo build --package tugboat-cloud-hypervisor-runtime` が成功する
- [ ] `cargo clippy --package tugboat-cloud-hypervisor-runtime` がパスする
- [ ] `cargo fmt --check` がパスする
- [ ] `cargo test --package tugboat-cloud-hypervisor-runtime` がパスする（validate.rs のテスト含む）
- [ ] 全サブコマンドがstub（`todo!()`）として存在する
- [ ] `CloudHypervisorVmConfig` が定義され、TOMLからデシリアライズ可能

## テスト・確認方法

```bash
cargo build --package tugboat-cloud-hypervisor-runtime
cargo clippy --package tugboat-cloud-hypervisor-runtime
cargo fmt --check
cargo test --package tugboat-cloud-hypervisor-runtime
```
