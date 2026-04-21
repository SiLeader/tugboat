# Task 3: VM起動（executeモジュール）とcreate/start/runサブコマンド

## 目的

Cloud HypervisorのCLI引数を構築して `exec()` でハイパーバイザプロセスに置き換えるexecuteモジュールと、2フェーズライフサイクル（create/start）およびワンショット（run）のサブコマンドを実装する。

## 依存タスク

- Task 1（スキャフォールド）

## 実装内容

### 1. 作成するファイル

| ファイル | 内容 |
|---------|------|
| `src/execute/mod.rs` | `run()` エントリポイント: ユーザー切り替え + スポナー呼び出し |
| `src/execute/vm/mod.rs` | `RunVm`, `Spawner` トレイト定義 |
| `src/execute/vm/cloud_hypervisor/mod.rs` | CLI引数構築、`exec()` |
| `src/execute/vm/cloud_hypervisor/spawner.rs` | `CloudHypervisorVmConfig`, `CloudHypervisorVmBuilder` |
| `src/execute/vm/cloud_hypervisor/volume_copy.rs` | ブートディスクコピー |
| `src/cmd/create/mod.rs` | 2フェーズ作成（Phase 1） |
| `src/cmd/start/mod.rs` | 2フェーズ作成（Phase 2: FIFOアンブロック） |
| `src/cmd/run/mod.rs` | ワンショット実行 |

### 2. Cloud Hypervisor CLI引数マッピング

`VmRunRequest` からCloud HypervisorのCLI引数への変換:

| VmRunRequest フィールド | Cloud Hypervisor CLI引数 |
|------------------------|------------------------|
| id | `--api-socket {disk_image_location}/{id}.ch.sock` |
| cpu.cores | `--cpus boot={cores}` |
| memory.size (bytes) | `--memory size={size}` |
| uefi.enabled=false + config.boot.kernel | `--kernel {kernel}` + `--initramfs {initramfs}` |
| uefi.enabled=true + config.boot.firmware | `--firmware {firmware}` |
| networks[i] | `--net tap={iface_name},mac={mac_address}` |
| volumes[i] (Block) | `--disk path={host_path},readonly={on\|off}` |
| volumes[i] (Filesystem) | `--fs tag={mount_tag},socket=/tmp/virtiofsd-{id}-{n}.sock` (注: virtiofsdの設定が別途必要) |
| ブートディスク | `--disk path={disk_image_location}/{id}.img` (最初のディスクとして) |
| incoming (マイグレーション受信) | API経由で設定（CLI引数なし、起動後に `/api/v1/vm.receive-migration` を呼ぶ） |
| - | `--serial tty --console off` (nographic相当) |

### 3. CloudHypervisorVmConfig パスメソッド

```rust
impl CloudHypervisorVmConfig {
    pub fn get_api_socket_path(&self, id: &str) -> String {
        format!("{}/{}.ch.sock", self.disk_image_location, id)
    }
}
```

### 4. ブートディスク処理

- ソースイメージを `{disk_image_location}/{id}.img` にコピー（`tokio::fs::copy`）
- Cloud Hypervisorはraw、qcow2の両方をサポート

### 5. create/start ライフサイクル

QEMU runtimeと同一パターン:
1. `create`: VmRunRequest読み込み → マウントNS入場 → ネットワークNS作成・入場 → デーモン化 → FIFO作成・待機 → `exec()` でcloud-hypervisor起動
2. `start`: IDのバリデーション → FIFOに書き込み（ブロック解除）

### 6. 入力バリデーション

- `validate_safe_id()` をVM IDに適用
- Cloud Hypervisorはカンマ区切りのkey=valueオプションを使用するため、値に `,` や `=` が含まれないことを検証する関数を追加（`validate_ch_option_value`）

### 7. 参照ファイル

- `tugboat-qemu-runtime/src/execute/vm/qemu/mod.rs` (307行): CLI引数構築パターン
- `tugboat-qemu-runtime/src/execute/vm/qemu/spawner.rs` (73行): Config + Builder パターン
- `tugboat-qemu-runtime/src/execute/vm/qemu/volume_copy.rs` (35行): ブートディスクコピー
- `tugboat-qemu-runtime/src/cmd/create/mod.rs` (40行): createフロー
- `tugboat-qemu-runtime/src/cmd/start/mod.rs` (27行): startフロー
- `tugboat-qemu-runtime/src/cmd/run/mod.rs` (35行): runフロー
- `tugboat-qemu-runtime/src/execute/mod.rs` (18行): executeエントリポイント
- `tugboat-qemu-runtime/src/execute/vm/mod.rs` (29行): トレイト定義

## 受け入れ条件

- [ ] Cloud HypervisorのCLI引数が `VmRunRequest` から正しく構築される
- [ ] CPU、メモリ、ディスク、ネットワーク、カーネル/UEFIブートの各引数が正しい
- [ ] ブートディスクが正しくコピーされる
- [ ] create/start の2フェーズライフサイクルが動作する（FIFOベース）
- [ ] runのワンショット実行が動作する
- [ ] 入力バリデーション（safe ID、オプション値インジェクション防止）が機能する
- [ ] `cargo test` がパスする

## テスト・確認方法

ユニットテスト:
- CLI引数構築のテスト（各フィールドの変換が正しいか）
- ブートモード選択テスト（uefi.enabled による kernel/firmware 切り替え）
- バリデーション関数のテスト

```bash
cargo test --package tugboat-cloud-hypervisor-runtime
cargo clippy --package tugboat-cloud-hypervisor-runtime
```
