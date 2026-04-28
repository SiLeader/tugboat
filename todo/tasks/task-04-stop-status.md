# Task 4: stop / status サブコマンド

## 目的

Cloud Hypervisor REST API経由でVMの停止（shutdown/poweroff）と状態クエリを実装する。

## 依存タスク

- Task 1（スキャフォールド）
- Task 2（APIクライアント）

## 実装内容

### 1. 作成するファイル

| ファイル | 内容 |
|---------|------|
| `src/cmd/stop/mod.rs` | VM停止サブコマンド |
| `src/cmd/status/mod.rs` | VM状態取得サブコマンド |

### 2. stop サブコマンド

`VmStopRequest` を読み取り、停止タイプに応じてAPIを呼ぶ:

| VmStopType | Cloud Hypervisor API | 説明 |
|-----------|---------------------|------|
| `Shutdown` | `PUT /api/v1/vm.power-button` | ACPI電源ボタン（ゲストOSが正常シャットダウン） |
| `PowerOff` | `PUT /api/v1/vm.shutdown` | 即時シャットダウン |

実装フロー:
1. `VmStopRequest` をstdinからJSON読み取り
2. `validate_safe_id` でIDをバリデーション
3. APIソケットパスを取得 (`{disk_image_location}/{id}.ch.sock`)
4. `ChApiClient::connect()` で接続
5. 停止タイプに応じてAPIを呼び出し

### 3. status サブコマンド

`GET /api/v1/vm.info` を呼び出し、`VmStatusResponse` をstdoutにJSON出力。

Cloud Hypervisor の状態マッピング:

| Cloud Hypervisor state | VmStatus | 説明 |
|----------------------|----------|------|
| `"Created"` | `Prelaunch` | VM作成済み、未起動 |
| `"Running"` | `Running` | 実行中 |
| `"Shutdown"` | `Shutdown` | シャットダウン済み |
| `"Paused"` | `Paused` | 一時停止中 |
| `"BreakPoint"` | `Paused` | ブレークポイント（デバッグ） |

実装フロー:
1. VM IDを引数から取得
2. `validate_safe_id` でIDをバリデーション
3. APIソケットパスを取得
4. `ChApiClient::connect()` で接続
5. `GET /api/v1/vm.info` を実行
6. レスポンスから `state` フィールドを取得
7. `VmStatusResponse` を構築してstdoutにJSON出力

### 4. 参照ファイル

- `tugboat-qemu-runtime/src/cmd/stop/mod.rs` (52行): QMP版のstop実装
- `tugboat-qemu-runtime/src/cmd/status/mod.rs` (91行): QMP版のstatus実装
- `tugboat-qemu-runtime/src/cmd/qmp.rs` (54行): 接続ユーティリティパターン

## 受け入れ条件

- [ ] `Shutdown` タイプで `PUT /api/v1/vm.power-button` が呼ばれる
- [ ] `PowerOff` タイプで `PUT /api/v1/vm.shutdown` が呼ばれる
- [ ] Cloud Hypervisorの全状態が正しく `VmStatus` にマッピングされる
- [ ] `VmStatusResponse` がJSON形式でstdoutに出力される
- [ ] APIソケット接続失敗時に適切なエラーが返される
- [ ] `cargo test` がパスする

## テスト・確認方法

ユニットテスト（モックHTTPサーバー使用）:

```rust
#[tokio::test]
async fn test_stop_shutdown_calls_power_button() {
    // モックサーバーがPUT /api/v1/vm.power-buttonを受信することを検証
}

#[tokio::test]
async fn test_stop_poweroff_calls_shutdown() {
    // モックサーバーがPUT /api/v1/vm.shutdownを受信することを検証
}

#[tokio::test]
async fn test_status_running() {
    // vm.infoが{"state":"Running"}を返した場合にVmStatus::Runningが出力されることを検証
}

#[tokio::test]
async fn test_status_all_states() {
    // 全状態マッピングを検証
}
```

```bash
cargo test --package tugboat-cloud-hypervisor-runtime
cargo clippy --package tugboat-cloud-hypervisor-runtime
```
