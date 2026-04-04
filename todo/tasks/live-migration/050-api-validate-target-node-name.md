# API レベルでの targetNodeName バリデーション追加

## 概要

Ship の `spec.targetNodeName` は PUT/PATCH で自由に設定できるが、API サーバ側での
バリデーションが行われていない。現状は agent の preflight までエラーが届かないため、
明らかに不正な値（`nodeName` と同一など）を早期に拒否できない。

APIサーバの Ship 更新処理に簡易なバリデーションを追加する。

## 対象ファイル

- `tugboat-apiserver/src/endpoints/v1_core/ship.rs`（または共通バリデーション層）
- `tugboat-apiserver/src/data/create.rs` または `tugboat-apiserver/src/data/mod.rs`

## 実装内容

Ship の PUT / PATCH (`handle_ship_replace`, `handle_ship_patch`) 処理において:

1. `spec.targetNodeName` が設定されている場合:
   - `spec.targetNodeName == spec.nodeName` ならば 400 Bad Request を返す
   - `spec.targetNodeName` が空文字列であれば 400 Bad Request を返す

2. マイグレーション中（`status.migration.phase` が `Pending` / `Ready` / `Migrating`）に
   `spec.targetNodeName` を別の値に変更しようとした場合は 409 Conflict を返す
   （中断は `/migrate/abort` エンドポイントを使用すること）

## 完了条件

- `nodeName` と同じ `targetNodeName` を設定すると 400 が返る
- マイグレーション中に `targetNodeName` を変更しようとすると 409 が返る
- `cargo test` が通ること
