# 11: ServiceAccountトークンコントローラー

## 概要

ServiceAccountが作成された際に、対応する認証トークン (Secret) を自動生成するコントローラーを実装する。

## 設計

### 動作

1. ServiceAccountの作成を監視 (watch)
2. 新しいServiceAccountが作成されたら:
   - トークン用のSecretを自動生成
   - Secret の type を `tugboat.io/service-account-token` に設定
   - Secret にトークン文字列を格納
   - ServiceAccount の `secrets` フィールドにSecretへの参照を追加
3. ServiceAccountが削除されたら対応するSecretも削除

### Secret構造

```json
{
  "metadata": {
    "name": "<serviceaccount-name>-token-<random>",
    "namespace": "<namespace>",
    "annotations": {
      "tugboat.io/service-account.name": "<serviceaccount-name>"
    }
  },
  "type": "tugboat.io/service-account-token",
  "data": {
    "token": "<generated-jwt-or-opaque-token>"
  }
}
```

## 作業内容

1. `tugboat-controller-manager/src/controllers/` に `service_account_token_controller.rs` を追加
2. トークン生成ロジック (JWT or opaque token)
3. コントローラーのreconcileループ実装
4. `tugboat-controller-manager/src/main.rs` にコントローラーを登録

## 確認

```bash
cargo build --package tugboat-controller-manager
cargo test --package tugboat-controller-manager
```

## 参考

- 既存コントローラー: `tugboat-controller-manager/src/controllers/` のパターン
- Kubernetes TokenController の動作
