# ラベルセレクターとフィールドセレクターの結合テスト

## 概要

一覧取得 API のラベルセレクター（`labelSelector`）とフィールドセレクター（`fieldSelector`）
によるフィルタリングが正しく動作することを検証する。

## テストシナリオ

### シナリオ 1: ラベルセレクターによるフィルタリング

1. 以下のラベルを持つ Ship を作成:
   - `app=web, tier=frontend`
   - `app=web, tier=backend`
   - `app=db, tier=backend`
2. `labelSelector=app=web` で一覧取得 → 2 件返ること
3. `labelSelector=tier=backend` で一覧取得 → 2 件返ること
4. `labelSelector=app=web,tier=frontend` で一覧取得 → 1 件返ること
5. `labelSelector=app=nonexistent` で一覧取得 → 0 件返ること

### シナリオ 2: フィールドセレクターによるフィルタリング

1. 複数の Ship を作成（異なる `metadata.name`）
2. `fieldSelector=metadata.name=<specific-name>` で一覧取得 → 1 件返ること
3. `fieldSelector=metadata.namespace=<specific-ns>` でフィルタ

### シナリオ 3: クラスタスコープリソースでのセレクター

1. 異なるラベルを持つ Node を複数作成
2. ラベルセレクターで特定の Node のみが返ることを確認

### シナリオ 4: 全ネームスペース一覧でのセレクター

1. 複数 Namespace に異なるラベルの Ship を作成
2. ネームスペース指定なしの一覧 API でラベルセレクターを使用
3. ネームスペースを跨いで正しくフィルタされることを確認

## 完了条件

- ラベルセレクターとフィールドセレクターの各パターンがテストされていること
- クラスタスコープ・ネームスペーススコープ両方でテストされていること
- `cargo test` が通ること
