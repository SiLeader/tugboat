# タスク 03: QEMU ホットプラグコマンド実装

## 前提タスク

- タスク 01 (Runtime Interface 型定義) が完了していること

## 目的

`tugboat-runtime/src/cmd/hotplug/mod.rs` を新規作成し、QEMU の QMP ソケット経由で
CPU・メモリ・NIC・ストレージデバイスのホットプラグ操作を実行するコマンドを実装する。

## 実装内容

### 新規ファイル: `tugboat-runtime/src/cmd/hotplug/mod.rs`

既存の `migrate/mod.rs` と同様に、stdin から `VmHotplugRequest` を読み込み、
QMP ソケット経由でホットプラグを実行する。

#### QMP ソケットパス

既存実装に倣い `QemuVmConfig::get_uds_url(&id)` でソケットパスを取得する。

#### CPU ホットプラグ

```
// ホットアド: vCPU の追加
// 現在のコア数と target cores の差分だけ cpu-add を発行する
// QMP コマンド例: {"execute":"device_add","arguments":{"driver":"qemu64-x86_64-cpu","id":"cpu-N"}}
```

- QEMU machine q35 + KVM での `device_add` (driver: `host-x86_64-cpu` または architecture に応じたドライバ) を使用
- 削除は `device_del` を使用

#### メモリ ホットプラグ

```
// pc-dimm デバイスを追加
// QMP: {"execute":"object-add","arguments":{"qom-type":"memory-backend-ram","id":"mem-N","size":SIZE}}
//       {"execute":"device_add","arguments":{"driver":"pc-dimm","id":"dimm-N","memdev":"mem-N"}}
```

- 削除は `device_del` + `object-del` を使用

#### NIC ホットプラグ

```
// QMP: {"execute":"netdev_add","arguments":{"type":"tap","id":"net-ID","ifname":"IFACE"}}
//       {"execute":"device_add","arguments":{"driver":"virtio-net-pci","netdev":"net-ID","id":"nic-ID","mac":"MAC"}}
```

- 削除は `device_del` + `netdev_del` を使用

#### ストレージ ホットプラグ

```
// QMP: {"execute":"blockdev-add","arguments":{"driver":"raw","node-name":"blk-ID","file":{"driver":"file","filename":"PATH"}}}
//       {"execute":"device_add","arguments":{"driver":"virtio-blk-pci","drive":"blk-ID","id":"dev-ID"}}
```

- 削除は `device_del` + `blockdev-del` を使用

#### QMP 通信の実装

QMP はまず `{"execute":"qmp_capabilities"}` でネゴシエーションが必要。
既存コードに QMP クライアントがなければ、Unix ドメインソケットを使った最小限の実装を
このファイル内に作成する（`tokio::net::UnixStream` を使用）。

#### エラーハンドリング

- QMP コマンドが `{"error": ...}` を返した場合は `Err` に変換する
- 部分的に成功した場合（CPU は成功、NIC は失敗など）もエラーとして伝播させる

### `tugboat-runtime/src/cmd/mod.rs` の更新

```rust
pub mod hotplug;
```

を追加する。

### `tugboat-runtime/src/main.rs` の更新

`hotplug` サブコマンドを clap に追加し、`cmd::hotplug::run()` を呼び出す。
引数は他のコマンドと同様に config ファイルパスのみ（stdin から JSON を読む）。

## テスト

QMP 通信は実際の QEMU プロセスなしにはテストできないため、単体テストは QMP メッセージの
構築ロジック（JSON 生成部分）のみを対象とする:

```rust
#[test]
fn cpu_add_command_format() {
    // device_add の引数 JSON が期待するフォーマットであることを確認
}

#[test]
fn nic_add_commands_format() {
    // netdev_add + device_add の JSON が正しい形式であることを確認
}
```

統合テストは受け入れ条件の手動確認で代替する。
