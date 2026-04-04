# ReplicaSet

`ReplicaSet` は、指定された数の `Ship` (VM) レプリカが常に実行されていることを保証します。特定の数の同一の Ship の可用性を保証するために使用されます。

## 動作の仕組み

`ReplicaSet` は、管理対象の `Ship` を識別するためのセレクター、維持すべきレプリカ数、およびレプリカ数を満たすために新しく作成する `Ship` のデータを定義するテンプレートによって定義されます。

`ReplicaSet` コントローラーは、必要に応じて `Ship` を作成または削除することで、この定義を実現します。`ReplicaSet` が新しい `Ship` を作成する必要がある場合、その `Ship` テンプレートを使用します。

## マニフェストの例

```yaml
apiVersion: apps/v1
kind: ReplicaSet
metadata:
  name: sample-rs
  namespace: default
spec:
  replicas: 3
  selector:
    app: web
  shipTemplate:
    metadata:
      labels:
        app: web
    spec:
      shipClassName: small
      image: "my-registry.local/web-app:v1"
```

## 主要なフィールド

- `spec.replicas`: 希望するレプリカ数。デフォルトは 1 です。
- `spec.selector`: この `ReplicaSet` に属する `Ship` を識別するために使用されるラベルセレクター。
- `spec.shipTemplate`: 新しい `Ship` を作成するために使用されるテンプレート。`metadata` (ラベルとアノテーション) と `spec` (VM の設定) が含まれます。

## 更新戦略

`ReplicaSet` は主にローリングアップデートのために `Deployment` によって管理されますが、直接更新することも可能です。

Tugboat は、`tugboat.dev/update-strategy: all` アノテーションが存在する場合、`ReplicaSet` のインプレース更新をサポートします。このモードでは、`shipTemplate` が更新されると、`ReplicaSet` コントローラーは管理下の既存のすべての `Ship` を新しいテンプレートに一致するように一斉に更新します。このアノテーションが存在しない場合、`ReplicaSet` コントローラーは通常、ダウンタイムを最小限に抑える方法（例：1台ずつ）で更新を実行します。
