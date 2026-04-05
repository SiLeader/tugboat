# Namespace ライフサイクルの結合テスト

## 概要

Namespace の作成・取得・一覧取得を API サーバー経由で検証する。
Namespace はクラスタスコープのリソースであり、他のネームスペーススコープリソースの前提となる。

## テストシナリオ

### シナリオ 1: Namespace の作成と取得

1. `POST /api/v1/namespaces` で Namespace `test-ns` を作成
2. レスポンスのステータスコードが 201 であること
3. `GET /api/v1/namespaces/test-ns` で取得
4. `metadata.name` が `test-ns` であること
5. `metadata.uid` が設定されていること
6. `metadata.creationTimestamp` が設定されていること

### シナリオ 2: Namespace の一覧取得

1. 複数の Namespace（`ns-a`, `ns-b`, `ns-c`）を作成
2. `GET /api/v1/namespaces` で一覧取得
3. 作成した 3 つの Namespace が全て含まれること

### シナリオ 3: 重複作成の拒否

1. Namespace `dup-ns` を作成
2. 同名の Namespace を再度作成
3. ステータスコードが 409 (Conflict) であること

### シナリオ 4: 存在しない Namespace の取得

1. `GET /api/v1/namespaces/nonexistent` を呼び出す
2. ステータスコードが 404 であること

## 完了条件

- 全シナリオのテストが実装されていること
- `cargo test` が通ること
