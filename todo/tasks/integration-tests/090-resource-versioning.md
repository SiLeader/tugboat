# リソースバージョニングと競合検出の結合テスト

## 概要

リソースの `resourceVersion` を使った楽観的同時実行制御が正しく機能することを検証する。
同一リソースの同時更新による競合を検出できることを確認する。

## テストシナリオ

### シナリオ 1: resourceVersion の自動付与

1. Ship を作成
2. 取得し `metadata.resourceVersion` が設定されていることを確認
3. パッチで更新
4. 再取得し `resourceVersion` が変更されていることを確認

### シナリオ 2: 古い resourceVersion での更新拒否

1. Ship を作成し取得（resourceVersion = v1）
2. Ship を別途パッチで更新（resourceVersion が v2 に変わる）
3. v1 の resourceVersion を使って PUT で置換を試みる
4. 409 Conflict が返ることを確認

### シナリオ 3: generation フィールドの更新

1. Ship を作成
2. spec を変更するパッチを送信
3. `metadata.generation` がインクリメントされていることを確認
4. metadata のみの変更（ラベル追加）では generation が変わらないことを確認（実装依存）

### シナリオ 4: uid の一意性

1. Ship を作成し uid を記録
2. 削除
3. 同名の Ship を再作成
4. uid が以前と異なることを確認

## 完了条件

- resourceVersion による競合検出がテストされていること
- uid の一意性がテストされていること
- `cargo test` が通ること
