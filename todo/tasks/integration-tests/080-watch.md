# Watch 機能の結合テスト

## 概要

リソースの変更をリアルタイムに監視する Watch 機能を検証する。
Watch は `list` エンドポイントに `watch=True` または `watch=NdJson` パラメータを付与して使用する。

## テストシナリオ

### シナリオ 1: リソース作成の Watch

1. Ship の Watch を開始（バックグラウンドタスクとして）
2. Ship を作成
3. Watch ストリームに `ADDED` イベントが届くことを確認
4. イベントのオブジェクトが作成した Ship と一致すること

### シナリオ 2: リソース更新の Watch

1. Ship を作成
2. Watch を開始
3. Ship をパッチで更新
4. Watch ストリームに `MODIFIED` イベントが届くことを確認

### シナリオ 3: リソース削除の Watch

1. Ship を作成
2. Watch を開始
3. Ship を削除
4. Watch ストリームに `DELETED` イベントが届くことを確認

### シナリオ 4: resourceVersion からの Watch

1. Ship を作成し resourceVersion を記録
2. Ship を更新
3. 記録した resourceVersion を指定して Watch を開始
4. 更新イベントのみが届くことを確認（作成イベントは含まれない）

### シナリオ 5: ラベルセレクター付き Watch

1. ラベル `app=watched` を持つ Ship の Watch を開始
2. ラベルなしの Ship を作成 → Watch にイベントが届かないこと
3. `app=watched` ラベルの Ship を作成 → Watch にイベントが届くこと

### シナリオ 6: NdJson 形式の Watch

1. `watch=NdJson` で Watch を開始
2. リソースを作成
3. Newline-delimited JSON 形式でイベントが返ることを確認

## 完了条件

- 作成・更新・削除の各 Watch イベントがテストされていること
- resourceVersion 指定の Watch がテストされていること
- セレクター付き Watch がテストされていること
- `cargo test` が通ること
