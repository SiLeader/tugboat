# 12: Namespace作成時のデフォルトServiceAccount自動生成

## 概要

Namespaceが作成された際に、`default` ServiceAccountを自動生成するコントローラーを実装する。
Kubernetesと同様に、全Namespaceにデフォルトの認証IDを提供する。

## 設計

### 動作

1. Namespaceの作成を監視 (watch)
2. 新しいNamespaceが作成されたら:
   - そのNamespace内に `default` という名前のServiceAccountを作成
   - 既に存在する場合はスキップ
3. タスク11のServiceAccountトークンコントローラーが自動的にトークンSecretを生成

## 作業内容

1. `tugboat-controller-manager/src/controllers/` に `namespace_default_sa_controller.rs` を追加
   - または既存のNamespace関連コントローラーに統合
2. reconcileループ実装
3. `tugboat-controller-manager/src/main.rs` にコントローラーを登録

## 確認

```bash
cargo build --package tugboat-controller-manager
cargo test --package tugboat-controller-manager
```

## 参考

- Kubernetes ServiceAccount Admission Controller の動作
- 既存のNamespaceリソース処理
