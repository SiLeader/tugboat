# タスク: ShipSpec への runtime_class フィールド追加

## 前提

タスク `01-proto-definition.md` が完了済みであること。

## 概要

`tugboat-resources/proto/core/v1/ship.proto` の `ShipSpec` メッセージに `runtime_class` フィールドを追加し、
Ship がどの `RuntimeClass` を必要とするかを指定できるようにする。

## 実装内容

### ship.proto の更新

`ShipSpec` メッセージに以下のフィールドを追加する（フィールド番号は現在の最大値 `10` の次）:

```protobuf
optional string runtime_class = 11; // 参照する RuntimeClass 名
```

### 影響確認

`ship.proto` の変更はコード生成を通じて `ShipSpec` 構造体に `runtime_class: Option<String>` フィールドを追加する。
既存のコードがコンパイルエラーになっていないかを確認する（`ShipSpec` の構造体リテラルを使用している箇所がある場合は `..Default::default()` で補完されるため基本的に影響なし）。

## 確認コマンド

```bash
cargo build
cargo clippy
```

`ShipSpec` を構造体リテラルで初期化している箇所がある場合は `Default::default()` を使ったフィールド補完が必要になる可能性があるため、ビルドエラーに注意する。
