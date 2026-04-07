# タスク 01: Runtime Interface ホットプラグ型定義

## 目的

`tugboat-vm-runtime-interface/src/hotplug.rs` を新規作成し、ホットプラグ操作に使用する
リクエスト型を定義する。これは後続タスク（QEMU 実装・Agent 拡張）すべての基盤となる。

## 実装内容

### 新規ファイル: `tugboat-vm-runtime-interface/src/hotplug.rs`

以下の型を定義する:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmHotplugRequest {
    pub id: String,
    pub cpu: Option<VmCpuHotplugConfig>,
    pub memory: Option<VmMemoryHotplugConfig>,
    pub nics_added: Vec<VmNetworkConfig>,     // VmNetworkConfig は run.rs から流用
    pub nics_removed: Vec<String>,            // netdev ID（"net-{mac_address}"形式など）
    pub volumes_added: Vec<VmVolumeConfig>,   // VmVolumeConfig は run.rs から流用
    pub volumes_removed: Vec<String>,         // blockdev ID
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmCpuHotplugConfig {
    pub cores: u64,  // 変更後の合計 vCPU 数
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmMemoryHotplugConfig {
    pub size: u64,  // 変更後の合計メモリ量（バイト）
}
```

### `tugboat-vm-runtime-interface/src/lib.rs` の更新

`pub mod hotplug;` を追加する。

## 注意点

- `VmNetworkConfig` と `VmVolumeConfig` は既存の `run.rs` から `use` するだけでよい
- `serde` の `rename_all = "camelCase"` を揃えて他の型と一貫性を保つこと
- `nics_removed` と `volumes_removed` はデバイス識別に使う文字列 ID とする
  （実装詳細は QEMU コマンド実装タスクで決定）

## テスト

`hotplug.rs` の `#[cfg(test)]` ブロックで以下を確認:

- `VmHotplugRequest` が JSON に正しくシリアライズ・デシリアライズできること
- `cpu: None`、`memory: None` でも正常に扱えること（空のホットプラグリクエスト）

```rust
#[test]
fn serialize_round_trip() {
    let req = VmHotplugRequest {
        id: "test-vm".to_string(),
        cpu: Some(VmCpuHotplugConfig { cores: 4 }),
        memory: None,
        nics_added: vec![],
        nics_removed: vec![],
        volumes_added: vec![],
        volumes_removed: vec![],
    };
    let json = serde_json::to_string(&req).unwrap();
    let decoded: VmHotplugRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, "test-vm");
    assert_eq!(decoded.cpu.unwrap().cores, 4);
    assert!(decoded.memory.is_none());
}
```
