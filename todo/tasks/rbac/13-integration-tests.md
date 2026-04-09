# 13: RBAC統合テスト

## 概要

RBAC機能全体の統合テストを実装する。

## テストケース

### RBACリソースCRUD

1. **Role CRUD**: Role の作成・一覧・取得・更新・パッチ・削除
2. **ClusterRole CRUD**: ClusterRole の作成・一覧・取得・更新・パッチ・削除
3. **RoleBinding CRUD**: RoleBinding の作成・一覧・取得・更新・パッチ・削除
4. **ClusterRoleBinding CRUD**: ClusterRoleBinding の作成・一覧・取得・更新・パッチ・削除
5. **ServiceAccount CRUD**: ServiceAccount の作成・一覧・取得・更新・パッチ・削除

### 認証テスト

6. **Bearer Token認証**: 正しいトークンでリクエスト → 認証成功
7. **無効なトークン**: 不正なトークンでリクエスト → 401 Unauthorized
8. **Anonymous access**: 認証情報なしでリクエスト → anonymous userとして処理

### 認可テスト

9. **基本的なRBAC許可**: Role + RoleBinding を作成 → バインドされたユーザーがリソースにアクセスできる
10. **RBAC拒否**: バインドされていないユーザーがリソースにアクセス → 403 Forbidden
11. **ClusterRole + ClusterRoleBinding**: クラスタスコープの権限が全Namespaceで有効
12. **ClusterRole + RoleBinding**: ClusterRoleをRoleBindingでバインド → 特定Namespaceのみで有効
13. **ワイルドカード**: `"*"` を使った PolicyRule のマッチ
14. **resource_names制限**: 特定のリソース名のみに制限
15. **system:masters バイパス**: system:mastersグループのユーザーは全アクセス可能

### コントローラーテスト

16. **ServiceAccountトークン自動生成**: ServiceAccount作成 → Secretが自動生成される
17. **Namespace default SA**: Namespace作成 → default ServiceAccountが自動生成される

### APIディスカバリーテスト

18. **RBAC API discovery**: `/apis/rbac.authorization/v1` でリソース一覧が返る

## 作業内容

1. `tests/integration/tests/rbac_resources_crud.rs` を作成
2. `tests/integration/tests/rbac_authorization.rs` を作成
3. `tests/integration/tests/rbac_controllers.rs` を作成
4. テストヘルパー: 認証付きHTTPクライアントの作成

## 確認

```bash
cargo test --package integration-tests
```

## 参考

- 既存テスト: `tests/integration/tests/namespaced_resources_crud.rs`
- 既存テスト: `tests/integration/tests/cluster_scoped_resources.rs`
