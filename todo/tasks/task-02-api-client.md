# Task 2: Cloud Hypervisor REST APIクライアント

## 目的

Cloud HypervisorのHTTP REST API（Unixドメインソケット経由）と通信する `ChApiClient` を実装する。QEMU runtimeの `QmpClient`（`cmd/hotplug/mod.rs:557-660`）に相当するコンポーネント。

## 依存タスク

- Task 1（スキャフォールド）

## 実装内容

### 1. 作成するファイル

- `src/cmd/api.rs` -- `ChApiClient` 実装

### 2. ChApiClient 設計

```rust
pub(crate) struct ChApiClient {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
}

impl ChApiClient {
    /// APIソケットに接続（5秒タイムアウト）
    pub async fn connect(socket_path: impl AsRef<Path>) -> crate::Result<Self>;

    /// GET リクエスト
    pub async fn get(&mut self, path: &str) -> crate::Result<serde_json::Value>;

    /// PUT リクエスト（JSONボディ付き）
    pub async fn put(&mut self, path: &str, body: Option<&serde_json::Value>) -> crate::Result<Option<serde_json::Value>>;
}
```

### 3. HTTP/1.1 プロトコル実装

リクエスト送信:
```
PUT /api/v1/vm.shutdown HTTP/1.1\r\n
Host: localhost\r\n
Content-Type: application/json\r\n
Content-Length: {len}\r\n
\r\n
{json_body}
```

レスポンス解析:
- ステータスライン解析 (`HTTP/1.1 200 OK`)
- ヘッダー解析 (`Content-Length` の取得)
- ボディ読み取りとJSON解析
- 非2xxステータスの場合は `Error::Api` を返す

### 4. ヘルパー関数

```rust
/// APIソケットのパスを取得
pub fn get_api_socket_path(config: &CloudHypervisorVmConfig, id: &str) -> String {
    format!("{}/{}.ch.sock", config.disk_image_location, id)
}
```

### 5. 使用するCloud Hypervisor APIエンドポイント

| エンドポイント | メソッド | 用途 | 使用タスク |
|--------------|---------|------|-----------|
| `/api/v1/vm.info` | GET | VM状態取得 | Task 4, 6 |
| `/api/v1/vm.shutdown` | PUT | 即時シャットダウン | Task 4 |
| `/api/v1/vm.power-button` | PUT | ACPI電源ボタン（グレースフル） | Task 4 |
| `/api/v1/vm.resize` | PUT | CPU/メモリリサイズ | Task 5 |
| `/api/v1/vm.add-net` | PUT | NIC追加 | Task 5 |
| `/api/v1/vm.add-disk` | PUT | ディスク追加 | Task 5 |
| `/api/v1/vm.remove-device` | PUT | デバイス削除 | Task 5 |
| `/api/v1/vm.send-migration` | PUT | ライブマイグレーション開始 | Task 6 |
| `/api/v1/vm.receive-migration` | PUT | マイグレーション受信準備 | Task 6 |

### 6. 参照ファイル

- `tugboat-qemu-runtime/src/cmd/hotplug/mod.rs:557-660` -- `QmpClient` 実装（パターンリファレンス）
- `tugboat-qemu-runtime/src/cmd/qmp.rs` -- QMP接続ユーティリティ（タイムアウトパターン）

## 受け入れ条件

- [ ] `ChApiClient` がUnixソケット経由でHTTP GET/PUT リクエストを送信できる
- [ ] 5秒の接続タイムアウトが実装されている
- [ ] 非2xxレスポンスが `Error::Api` に変換される
- [ ] `Content-Length` ベースのレスポンスボディ読み取りが動作する
- [ ] ユニットテストがモックHTTPサーバーで動作する

## テスト・確認方法

ユニットテスト（モックUnixソケットHTTPサーバー使用）:

```rust
#[tokio::test]
async fn test_get_request() {
    // Unixソケットでリッスンするモックサーバーを起動
    // ChApiClient::connect() で接続
    // GET リクエストを送信し、レスポンスを検証
}

#[tokio::test]
async fn test_put_request_with_body() {
    // PUT リクエスト + JSONボディの送受信を検証
}

#[tokio::test]
async fn test_error_response() {
    // 非2xxレスポンスが Error::Api になることを検証
}
```

```bash
cargo test --package tugboat-cloud-hypervisor-runtime
```
