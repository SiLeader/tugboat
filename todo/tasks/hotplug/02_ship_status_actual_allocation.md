# タスク 02: ShipStatus への actual_allocation フィールド追加

## 目的

`tugboat-resources/proto/core/v1/ship.proto` の `ShipStatus` に `actual_allocation` フィールドを追加し、
ホットプラグ後の実際のリソース割り当て状態を Ship リソース上で確認できるようにする。

## 実装内容

### `tugboat-resources/proto/core/v1/ship.proto` の変更

`ShipStatus` に以下を追加:

```protobuf
message ShipStatus {
  // ...既存フィールド...
  optional ShipActualAllocation actual_allocation = 4;
}

message ShipActualAllocation {
  optional uint64 cpu_cores = 1;
  optional string memory_size = 2;
  repeated string nic_ids = 3;
  repeated string volume_ids = 4;
}
```

- フィールド番号は既存フィールドと重複しないよう proto ファイルを確認してから割り当てる

### `tugboat-resources/build.rs` の確認

`ship.proto` がすでにビルド対象に含まれているか確認する。含まれていれば変更不要。

## ビルド確認

```bash
cargo build --package tugboat-resources
```

コンパイルエラーがないことを確認する。特に `ShipStatus` を使っている既存コードへの影響がないことを確認する
（`actual_allocation` は `optional` なので既存コードの破壊的変更はない）。

## テスト

特別な自動テストは不要。ビルド成功をもって確認とする。
ただし `cargo test --package tugboat-resources` を実行して既存テストが通ることを確認すること。
