# Task 5: hotplug サブコマンド（CPU / メモリ / NIC / ボリューム）

## 目的

Cloud Hypervisor REST API経由でCPU/メモリのリサイズ、NIC/ボリュームの追加・削除（ホットプラグ）をロールバック付きで実装する。

## 依存タスク

- Task 1（スキャフォールド）
- Task 2（APIクライアント）

## 実装内容

### 1. 作成するファイル

| ファイル | 内容 |
|---------|------|
| `src/cmd/hotplug/mod.rs` | ホットプラグ実装（メイン） |

### 2. ホットプラグ操作マッピング

#### CPU/メモリ リサイズ

Cloud Hypervisorは `PUT /api/v1/vm.resize` で一括リサイズ:

```json
{
    "desired_vcpus": 4,
    "desired_ram": 8589934592
}
```

- QEMUのようにスロット単位の追加/削除は不要
- CPU縮小・メモリ縮小も同じAPIで対応

#### NIC ホットプラグ

追加: `PUT /api/v1/vm.add-net`
```json
{
    "tap": "tap0",
    "mac": "AA:BB:CC:DD:EE:FF",
    "id": "net-{sanitized_key}"
}
```

削除: `PUT /api/v1/vm.remove-device`
```json
{
    "id": "net-{sanitized_key}"
}
```

IDの生成は `tugboat_vm_runtime_interface::hotplug::sanitize_identifier()` を使用（MACアドレスのSHA-256ハッシュ）。

#### ボリューム ホットプラグ

追加: `PUT /api/v1/vm.add-disk`
```json
{
    "path": "/dev/sdb",
    "readonly": false,
    "id": "blk-{sanitized_key}"
}
```

削除: `PUT /api/v1/vm.remove-device`
```json
{
    "id": "blk-{sanitized_key}"
}
```

IDの生成は `tugboat_vm_runtime_interface::hotplug::sanitize_identifier()` を使用（ホストパスのSHA-256ハッシュ）。

### 3. ロールバック機構

QEMU runtimeと同じパターン（`cmd/hotplug/mod.rs:38-116`）:

```rust
enum HotplugRollback {
    ResizeCpu { previous_vcpus: u64 },
    ResizeMemory { previous_ram: u64 },
    RemoveNic { id: String },
    AddNic { id: String },
    RemoveDisk { id: String },
    AddDisk { id: String },
}
```

実行フロー:
1. 各ホットプラグ操作を順次実行
2. 成功した操作のロールバックアクションを記録
3. 任意の操作が失敗した場合、記録された操作を逆順で実行してロールバック
4. ロールバック失敗はログ出力のみ（伝搬しない）

### 4. 実行順序

1. CPU リサイズ（指定時）
2. メモリ リサイズ（指定時）
3. NIC 削除（`nics_removed` の各項目）
4. NIC 追加（`nics_added` の各項目）
5. ボリューム削除（`volumes_removed` の各項目）
6. ボリューム追加（`volumes_added` の各項目）

### 5. 現在のリサイズ状態の取得

CPU/メモリのロールバックには現在値が必要。`GET /api/v1/vm.info` から取得:
- `config.cpus.boot` -> 現在のvCPU数
- `config.memory.size` -> 現在のメモリサイズ

### 6. 参照ファイル

- `tugboat-qemu-runtime/src/cmd/hotplug/mod.rs` (1085行): QMP版ホットプラグ実装（ロールバックパターン、テストインフラ）
- `tugboat-vm-runtime-interface/src/hotplug.rs` (66行): `VmHotplugRequest`, `sanitize_identifier()`, `normalize_identifier_key()`

## 受け入れ条件

- [ ] CPU/メモリリサイズが `PUT /api/v1/vm.resize` で正しく動作する
- [ ] NIC追加が `PUT /api/v1/vm.add-net` で正しく動作する
- [ ] NIC削除が `PUT /api/v1/vm.remove-device` で正しく動作する
- [ ] ボリューム追加が `PUT /api/v1/vm.add-disk` で正しく動作する
- [ ] ボリューム削除が `PUT /api/v1/vm.remove-device` で正しく動作する
- [ ] 操作失敗時にロールバックが実行される
- [ ] ロールバック失敗はログ出力のみで伝搬しない
- [ ] デバイスIDが `sanitize_identifier()` で正しく生成される
- [ ] `cargo test` がパスする

## テスト・確認方法

ユニットテスト（モックHTTPサーバー使用）:

```rust
#[tokio::test]
async fn test_cpu_resize() { /* vm.resize が正しいvCPU数で呼ばれることを検証 */ }

#[tokio::test]
async fn test_memory_resize() { /* vm.resize が正しいメモリサイズで呼ばれることを検証 */ }

#[tokio::test]
async fn test_nic_add() { /* vm.add-net が正しいパラメータで呼ばれることを検証 */ }

#[tokio::test]
async fn test_nic_remove() { /* vm.remove-device が正しいIDで呼ばれることを検証 */ }

#[tokio::test]
async fn test_volume_add() { /* vm.add-disk が正しいパラメータで呼ばれることを検証 */ }

#[tokio::test]
async fn test_volume_remove() { /* vm.remove-device が正しいIDで呼ばれることを検証 */ }

#[tokio::test]
async fn test_rollback_on_failure() {
    // NIC追加が失敗した場合、先に成功したCPUリサイズがロールバックされることを検証
}
```

```bash
cargo test --package tugboat-cloud-hypervisor-runtime
cargo clippy --package tugboat-cloud-hypervisor-runtime
```

### ファイルサイズ目安

QEMU版は1085行（カスタムQMPクライアント含む）。Cloud Hypervisor版はAPIクライアントが別モジュールにあり、リサイズAPIが単純なため、**400-600行程度**を目標とする（テスト含む）。800行を超える場合はテストを別ファイルに分離する。
