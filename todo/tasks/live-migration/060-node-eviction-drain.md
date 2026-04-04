# ノード Eviction（Drain）API の実装

## 概要

ノードをメンテナンスモードに入れる際、そのノード上の全 Ship を別のノードへライブマイグレーション
したい。現状は Ship を一つずつ手動で `spec.targetNodeName` を設定する必要があり手間がかかる。

ノード eviction エンドポイントを追加し、対象ノードの全 Ship に対して
スケジューラが選択した最適ノードへの移行を自動的に開始する。

## 対象ファイル

- `tugboat-apiserver/src/endpoints/v1_core/node.rs`
- `tugboat-apiserver/src/endpoints/v1_core/mod.rs`
- `tugboat-resources/proto/core/v1/node.proto`（NodeSpec への `unschedulable` フィールド追加）

## 実装内容

### エンドポイント

```
POST /api/v1/nodes/{name}/drain
```

### 処理フロー

1. 対象ノードの `spec.unschedulable` を `true` に設定（新規 Ship がスケジュールされないようにする）
2. そのノード上の全 Ship（`spec.nodeName == node_name` かつ `spec.targetNodeName` が未設定）を列挙
3. 各 Ship に対してスケジューラを呼び出し（または適切な別ノードを選択）、
   `spec.targetNodeName` を設定する
4. 開始した Ship 名・数をレスポンスに返す

### Node リソースへの `unschedulable` フィールド追加

`node.proto` の `NodeSpec` に `optional bool unschedulable = N;` を追加し、
スケジューラの `FilterPlugin` で `unschedulable == true` のノードを弾くよう対応する。

### 注意事項

- RWX 非対応の PVC を持つ Ship は drain 対象から除外し、レスポンスに警告を含める
- マイグレーション中の Ship は skip する
- ターゲットノードが見つからない場合は Ship を skip し、レスポンスに理由を含める

## 完了条件

- エンドポイントが実装されドレインが開始されること
- `unschedulable` ノードがスケジューラに弾かれること
- マイグレーション不可 Ship の skip と警告出力が実装されていること
- `cargo test` が通ること
