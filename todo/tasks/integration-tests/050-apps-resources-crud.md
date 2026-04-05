# Apps API リソースの CRUD 結合テスト

## 概要

Apps API グループ（Deployment, ReplicaSet, Fleet）の CRUD 操作を検証する。
これらは `/apis/apps/v1/` 配下のエンドポイントを使用する。

## テストシナリオ

### シナリオ 1: ReplicaSet の CRUD

1. Namespace を作成
2. `POST /apis/apps/v1/namespaces/{ns}/replicasets` で ReplicaSet を作成
   - `spec.replicas` と `spec.selector` と `spec.template` を設定
3. 取得し spec が正しいことを確認
4. パッチで `spec.replicas` を変更
5. 再取得し変更が反映されていることを確認
6. 削除

### シナリオ 2: Deployment の CRUD

1. `POST /apis/apps/v1/namespaces/{ns}/deployments` で Deployment を作成
   - `spec.replicas`, `spec.selector`, `spec.template`, `spec.strategy` を設定
2. 取得し内容を検証
3. パッチで `spec.replicas` を変更
4. 置換（PUT）で全体を更新
5. 削除

### シナリオ 3: Fleet の CRUD

1. Fleet を作成
2. 取得し内容を検証
3. パッチで更新
4. 削除

### シナリオ 4: 全ネームスペース横断一覧

1. 複数の Namespace に ReplicaSet/Deployment/Fleet を作成
2. ネームスペース指定なしの一覧 API で全てが返ることを確認

## 完了条件

- Deployment, ReplicaSet, Fleet の CRUD がテストされていること
- `cargo test` が通ること
