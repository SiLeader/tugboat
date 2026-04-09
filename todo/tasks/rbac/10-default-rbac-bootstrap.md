# 10: デフォルトRBACルールのブートストラップ

## 概要

APIサーバー起動時にデフォルトのClusterRole/ClusterRoleBindingを自動生成する。
Kubernetesと同様に、システム運用に必要な基本ロールを提供する。

## デフォルトClusterRole

### cluster-admin
全リソースに対する全操作を許可するスーパーユーザーロール。

```yaml
rules:
  - apiGroups: ["*"]
    resources: ["*"]
    verbs: ["*"]
```

### admin
namespace内の全リソースに対する全操作を許可。RoleBindingで利用。

```yaml
rules:
  - apiGroups: ["core", "apps"]
    resources: ["*"]
    verbs: ["*"]
  - apiGroups: ["authorization"]
    resources: ["roles", "rolebindings"]
    verbs: ["*"]
```

### edit
namespace内のリソースの読み書きを許可 (RBAC系は読み取りのみ)。

```yaml
rules:
  - apiGroups: ["core", "apps"]
    resources: ["*"]
    verbs: ["create", "get", "list", "watch", "update", "patch", "delete"]
  - apiGroups: ["authorization"]
    resources: ["roles", "rolebindings"]
    verbs: ["get", "list", "watch"]
```

### view
読み取り専用アクセス。

```yaml
rules:
  - apiGroups: ["core", "apps"]
    resources: ["*"]
    verbs: ["get", "list", "watch"]
```

## デフォルトClusterRoleBinding

### cluster-admin binding

```yaml
subjects:
  - kind: Group
    name: system:masters
    apiGroup: authorization
roleRef:
  apiGroup: authorization
  kind: ClusterRole
  name: cluster-admin
```

## 作業内容

1. `tugboat-apiserver/src/auth/bootstrap.rs` を作成
2. APIサーバー起動時にデフォルトのClusterRole/ClusterRoleBindingをetcdに書き込む
3. 既に存在する場合はスキップ (冪等性を保証)
4. `lib.rs` の起動シーケンスにブートストラップを追加

## 確認

```bash
cargo build --package tugboat-apiserver
cargo test --package tugboat-apiserver
```

## 参考

- Kubernetes default ClusterRoles: cluster-admin, admin, edit, view
- system:masters グループのバイパスルール
