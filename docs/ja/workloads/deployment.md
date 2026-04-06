# Deployment

`Deployment` は、`Ship` と `ReplicaSet` に対して宣言的な更新を提供します。`Deployment` で希望の状態を記述すると、
`Deployment` コントローラーは制御された速度で実際の状態を希望の状態に変更します。

## 動作の仕組み

`Deployment` は 1 つ以上の `ReplicaSet` を管理します。`Deployment` の `Ship` テンプレートを更新すると、新しい `ReplicaSet`
が作成され、古い `ReplicaSet` から新しい `ReplicaSet` へ徐々に `Ship` を移動させるか、変更内容と戦略に応じて既存の
`ReplicaSet` を更新します。

Tugboat では、2 つの更新パスを区別しています。

1. **ローテーションパス (Rotation Path)**: VM の再起動または再作成が必要な変更（例：VM イメージの変更）に使用されます。
2. **インプレースパス (In-place Path)**: 実行中の VM に適用可能な変更（例：エージェントがホットプラグで対応している場合の
   `shipClass` の変更）に使用されます。

## 更新戦略

### RollingUpdate (デフォルト)

`Deployment` は、新しい `ReplicaSet` を徐々にスケールアップし、古い `ReplicaSet` をスケールダウンすることで、古い
`ReplicaSet` を新しいものに置き換えます。

- `maxSurge`: 希望する `Ship` の数を超えて作成できる `Ship` の最大数。
- `maxUnavailable`: 更新プロセス中に利用不可にできる `Ship` の最大数。

**インプレースパス**では、`Deployment` は既存の `ReplicaSet` のテンプレートを更新し、`ReplicaSet` コントローラーは可用性を維持するために
`Ship` を 1 台ずつ更新します。

### Recreate

新しい `Ship` が作成される前に、既存のすべての `Ship` が停止されます。

- **ローテーションパス**: 古い `ReplicaSet` を 0 にスケールダウンし、すべての `Ship` が削除されるのを待ってから、新しい
  `ReplicaSet` をスケールアップします。
- **インプレースパス**: 既存の `ReplicaSet` のテンプレートを更新し、すべての `Ship` を一斉に更新するように指示します（
  `tugboat.cloud/update-strategy: all` アノテーションを使用）。

## マニフェストの例

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: web-server
  namespace: default
spec:
  replicas: 3
  selector:
    app: web
  strategy:
    type: RollingUpdate
    rollingUpdate:
      maxSurge: 1
      maxUnavailable: 1
  shipTemplate:
    metadata:
      labels:
        app: web
    spec:
      shipClassName: medium
      image: "my-registry.local/web-server:v2"
```

## ステータス

`DeploymentStatus` は以下を追跡します。

- `replicas`: このデプロイメントが対象とする、終了していない `Ship` の総数。
- `updatedReplicas`: 希望するテンプレートスペックを持つ、終了していない `Ship` の総数。
- `readyReplicas`: 準備ができている `Ship` の総数。
