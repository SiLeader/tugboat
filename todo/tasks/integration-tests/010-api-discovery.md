# API ディスカバリーエンドポイントの結合テスト

## 概要

API サーバーのディスカバリーエンドポイントが正しいレスポンスを返すことを検証する。
クライアントやツールがリソース一覧を正しく取得できることを保証する。

## 対象エンドポイント

- `GET /api` — コア API バージョン一覧
- `GET /api/v1` — コア v1 リソース一覧
- `GET /apis` — API グループ一覧
- `GET /apis/apps/v1` — apps/v1 リソース一覧
- `GET /apis/coordination/v1` — coordination/v1 リソース一覧
- `GET /healthz` — ヘルスチェック
- `GET /openapi/v3/api/v1` — OpenAPI スキーマ

## テストシナリオ

### シナリオ 1: コア API ディスカバリー

1. `GET /api` を呼び出す
2. レスポンスに `v1` が含まれることを確認

### シナリオ 2: リソース一覧の正確性

1. `GET /api/v1` を呼び出す
2. 以下のリソースが含まれることを確認:
   - `ships`, `namespaces`, `nodes`, `shipclasses`
   - `persistentvolumes`, `persistentvolumeclaims`
   - `configmaps`, `secrets`
   - `storageclasses`, `networkclasses`, `clusternetworkclasses`
3. 各リソースに `verbs` フィールドが存在し、少なくとも `create`, `list`, `get` を含むこと

### シナリオ 3: Apps API グループ

1. `GET /apis` を呼び出す
2. `apps` グループが含まれることを確認
3. `GET /apis/apps/v1` のリソースに `deployments`, `replicasets`, `fleets` が含まれること

### シナリオ 4: OpenAPI スキーマ

1. `GET /openapi/v3/api/v1` を呼び出す
2. 有効な JSON が返ること
3. Ship, Node 等のスキーマ定義が含まれること

## 完了条件

- 全ディスカバリーエンドポイントに対するテストが実装されていること
- `cargo test` が通ること
