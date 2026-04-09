# 08: 認可エバリュエーター (Authorization Evaluator)

## 概要

RBAC policyに基づいてリクエストの認可判定を行うエバリュエーターを実装する。

## 設計

### 認可リクエスト

```rust
pub struct AuthorizationRequest {
    pub user: UserInfo,
    pub verb: String,          // "create", "get", "list", "update", "patch", "delete", "watch"
    pub api_group: String,     // "core", "apps", "rbac.authorization"
    pub resource: String,      // "ships", "deployments", "roles"
    pub resource_name: Option<String>,
    pub namespace: Option<String>,
}
```

### 認可判定ロジック

1. ユーザーの `username` と `groups` を取得
2. **ClusterRoleBinding** を検索: ユーザー/グループにバインドされた ClusterRole を取得
3. namespaceが指定されている場合、**RoleBinding** を検索: そのnamespace内でユーザー/グループにバインドされた Role/ClusterRole を取得
4. 取得した全ての PolicyRule を評価:
   - `api_groups` にリクエストのapi_groupが含まれるか (`"*"` は全マッチ)
   - `resources` にリクエストのresourceが含まれるか (`"*"` は全マッチ)
   - `verbs` にリクエストのverbが含まれるか (`"*"` は全マッチ)
   - `resource_names` が空でない場合、リクエストのresource_nameが含まれるか
5. いずれかのruleがマッチすれば許可、いずれもマッチしなければ拒否

### Authorizer トレイト

```rust
#[async_trait]
pub trait Authorizer: Send + Sync {
    async fn authorize(&self, request: &AuthorizationRequest) -> AuthorizationDecision;
}

pub enum AuthorizationDecision {
    Allowed,
    Denied { reason: String },
}
```

### RbacAuthorizer

```rust
pub struct RbacAuthorizer {
    store: ResourceStore,
}
```

- etcdから Role/ClusterRole/RoleBinding/ClusterRoleBinding を読み取り、認可判定を行う
- パフォーマンス最適化のため、キャッシュの導入を検討 (初期実装では不要)

## 作業内容

1. `tugboat-apiserver/src/auth/` に追加:
   - `authorization.rs` - Authorizerトレイトと AuthorizationRequest/Decision
   - `rbac_authorizer.rs` - RBAC認可ロジック実装
2. ユニットテスト: PolicyRuleのマッチングロジック

## 確認

```bash
cargo build --package tugboat-apiserver
cargo test --package tugboat-apiserver
```

## 参考

- KubernetesのRBAC認可: https://kubernetes.io/docs/reference/access-authn-authz/rbac/
- PolicyRuleのワイルドカード (`"*"`) マッチ
- superuser (system:masters グループ) のバイパスルール
