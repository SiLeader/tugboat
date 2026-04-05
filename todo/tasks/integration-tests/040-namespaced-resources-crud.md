# ネームスペーススコープリソースの CRUD 結合テスト

## 概要

ネームスペーススコープリソース（Ship, ConfigMap, Secret, PVC, NetworkClass）の
CRUD 操作を API サーバー経由で検証する。

## 前提

- テスト用 Namespace が事前に作成されていること

## テストシナリオ

### シナリオ 1: Ship の CRUD

1. Namespace `test-ns` を作成
2. `POST /api/v1/namespaces/test-ns/ships` で Ship を作成
3. `GET /api/v1/namespaces/test-ns/ships/{name}` で取得し内容を検証
4. `PATCH /api/v1/namespaces/test-ns/ships/{name}` でアノテーションを追加
5. 再取得しアノテーションが反映されていることを確認
6. `PUT /api/v1/namespaces/test-ns/ships/{name}` で全体を置換
7. `DELETE /api/v1/namespaces/test-ns/ships/{name}` で削除

### シナリオ 2: Ship のステータス更新

1. Ship を作成
2. `PATCH /api/v1/namespaces/test-ns/ships/{name}/status` でステータスを更新
3. 取得しステータスが反映されていることを確認
4. `PUT /api/v1/namespaces/test-ns/ships/{name}/status` で置換

### シナリオ 3: ConfigMap の CRUD

1. ConfigMap を作成（data フィールドに key-value を設定）
2. 取得し data の内容を検証
3. パッチで data を更新
4. 削除

### シナリオ 4: Secret の CRUD

1. Secret を作成
2. 取得しデータが正しく保存されていることを確認
3. 削除

### シナリオ 5: PVC の CRUD とステータス

1. PersistentVolumeClaim を作成
2. 取得し spec を検証
3. ステータスをパッチで更新
4. 削除

### シナリオ 6: NetworkClass の CRUD とステータス

1. NetworkClass を作成
2. 取得し内容を検証
3. ステータスをパッチで更新
4. 削除

### シナリオ 7: 全ネームスペース横断一覧

1. Namespace `ns-a` と `ns-b` に同種のリソースを作成
2. `GET /api/v1/ships`（ネームスペース指定なし）で全ネームスペースの一覧を取得
3. 両方の Namespace のリソースが含まれることを確認

## 完了条件

- 各ネームスペーススコープリソースの CRUD がテストされていること
- 全ネームスペース横断一覧のテストが含まれること
- `cargo test` が通ること
