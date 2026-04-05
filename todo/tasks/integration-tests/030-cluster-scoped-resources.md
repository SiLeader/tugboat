# クラスタスコープリソースの CRUD 結合テスト

## 概要

クラスタスコープリソース（ShipClass, Node, StorageClass, ClusterNetworkClass）の
CRUD 操作を API サーバー経由で検証する。

## 対象リソース

- `ShipClass` — VM マシンタイプ定義
- `Node` — クラスタノード
- `StorageClass` — ストレージプロビジョニングポリシー
- `ClusterNetworkClass` — クラスタスコープのネットワーク設定

## テストシナリオ

### シナリオ 1: ShipClass の作成・取得・一覧

1. ShipClass `small` を作成（CPU, メモリ等のスペックを指定）
2. `GET /api/v1/shipclasses/small` で取得し内容を検証
3. 別の ShipClass `large` を作成
4. `GET /api/v1/shipclasses` で一覧取得し両方が含まれることを確認

### シナリオ 2: Node の CRUD

1. Node `node-1` を作成
2. `GET /api/v1/nodes/node-1` で取得
3. `PATCH /api/v1/nodes/node-1` でラベルを追加
4. 再度取得しラベルが反映されていることを確認
5. `DELETE /api/v1/nodes/node-1` で削除
6. 取得で 404 が返ることを確認

### シナリオ 3: Node のステータス更新

1. Node `status-node` を作成
2. `PATCH /api/v1/nodes/status-node/status` でステータスを更新
3. 取得しステータスが反映されていることを確認
4. `PUT /api/v1/nodes/status-node/status` でステータスを置換
5. 取得し置換後のステータスが反映されていることを確認

### シナリオ 4: StorageClass の作成と削除

1. StorageClass `fast-ssd` を作成
2. 取得し内容を検証
3. 削除し 404 になることを確認

### シナリオ 5: ClusterNetworkClass の作成とステータス

1. ClusterNetworkClass を作成
2. ステータスを更新
3. 取得しステータスが反映されていることを確認

## 完了条件

- 各リソースタイプの基本 CRUD 操作がテストされていること
- ステータスサブリソースの操作がテストされていること
- `cargo test` が通ること
