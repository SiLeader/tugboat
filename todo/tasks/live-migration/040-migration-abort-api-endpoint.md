# マイグレーション中断 API エンドポイントの追加

## 概要

現状、マイグレーションを中断または失敗後にリトライするには:

1. `PATCH /api/v1/namespaces/{ns}/ships/{name}` で `spec.targetNodeName: null` を設定
2. `PATCH /api/v1/namespaces/{ns}/ships/{name}/status` で `status.migration` をクリア

の 2 ステップが必要で、操作が複雑かつエラーが起きやすい。

専用の abort エンドポイントを追加し、一度の API 呼び出しでマイグレーションを安全に中断できるようにする。

## 対象ファイル

- `tugboat-apiserver/src/endpoints/v1_core/ship.rs`
- `tugboat-apiserver/src/endpoints/v1_core/mod.rs`
- `tugboat-apiserver/src/data/` (必要に応じてヘルパー追加)

## 実装内容

### エンドポイント

```
POST /api/v1/namespaces/{namespace}/ships/{name}/migrate/abort
```

### 処理内容

1. Ship を取得し、`spec.targetNodeName` が設定されているか確認。なければ 409 を返す
2. トランザクション的に以下を実行:
   - `spec.targetNodeName` を `null` にクリア
   - `status.migration.phase` を `"Failed"` に設定（または migration フィールド全体を削除）
   - `status.conditions` に中断イベントを追記
3. 更新後の Ship を返す

### 注意事項

- `PHASE_FAILED` または `PHASE_COMPLETED` の場合も中断操作を許容する（冪等に動作する）
- エージェントは `targetNodeName` が消えたことを検知し、target 側で起動済みの QEMU を削除する
  （`modify.rs` の既存ロジックがカバーするはず）

## 完了条件

- エンドポイントが実装されている
- `targetNodeName` がない Ship に対して 409 を返すこと
- OpenAPI スキーマに追記されていること
- `cargo test` が通ること
