# Lease リソースの CRUD 結合テスト

## 概要

Coordination API グループの Lease リソースの CRUD 操作を検証する。
Lease はリーダー選出に使用されるリソースである。

## テストシナリオ

### シナリオ 1: Lease の作成と取得

1. Namespace を作成
2. `POST /apis/coordination/v1/namespaces/{ns}/leases` で Lease を作成
   - `spec.holderIdentity`, `spec.leaseDurationSeconds` 等を設定
3. 取得し spec が正しいことを確認

### シナリオ 2: Lease の更新（リース更新シミュレーション）

1. Lease を作成
2. `PATCH` で `spec.renewTime` を更新
3. 再取得し更新されていることを確認
4. `PUT` で全体を置換

### シナリオ 3: 一覧取得

1. 複数の Lease を作成
2. ネームスペース内の一覧取得で全て含まれることを確認
3. 全ネームスペース横断一覧で含まれることを確認

## 完了条件

- Lease の CRUD 操作がテストされていること
- `cargo test` が通ること
