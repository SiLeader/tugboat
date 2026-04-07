# タスク 04: VmRuntimeOperator への hotplug メソッド追加

## 前提タスク

- タスク 01 (Runtime Interface 型定義) が完了していること
- タスク 03 (QEMU hotplug コマンド) が完了していること

## 目的

`tugboat-vm-runtime-interface/src/operator.rs` の `VmRuntimeOperator` に `hotplug` メソッドを追加し、
Agent から呼び出せるようにする。パターンは既存の `migrate`、`stop` と同じ。

## 実装内容

### `tugboat-vm-runtime-interface/src/operator.rs` の変更

```rust
use crate::hotplug::VmHotplugRequest;

impl VmRuntimeOperator {
    pub async fn hotplug(&self, args: VmHotplugRequest) -> Result<(), Error> {
        let child = self.call("hotplug", &args).await?;
        handle_command_response(child).await?;
        Ok(())
    }
}
```

`call` メソッドは既存の実装をそのまま使えるため、追加するのはこの1メソッドのみ。

## テスト

`VmRuntimeOperator` は実際のランタイムプロセスを起動するため単体テストは困難。
テストは以下で代替する:

- `cargo build --package tugboat-vm-runtime-interface` が通ること
- タスク 05（Agent 拡張）の実装時に呼び出し側からの結合で確認
