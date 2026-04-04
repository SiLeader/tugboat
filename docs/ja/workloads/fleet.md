# Fleet

`Fleet` は、関連しているが種類の異なる可能性のあるコンポーネントのグループを単一のユニットとして管理するために設計された、高レベルの抽象化です。これは、相互に通信する必要がある複数種類の VM で構成される複雑なアプリケーションに特に役立ちます。

## 主な機能

### コンポーネント管理

`Fleet` では複数の `components` を定義でき、それぞれが独自のレプリカ数と `Ship` テンプレートを持ちます。`Fleet` コントローラーは、各コンポーネントに対して `ReplicaSet` を自動的に作成し、管理します。

### 共有ネットワーク

`Fleet` の最も強力な機能の 1 つは、自動的な共有ネットワークです。`Fleet` スペックで `networkClassName` を指定すると、Tugboat は `Fleet` が管理するすべての `Ship` にこの `NetworkClass` を自動的に注入します。これにより、`Fleet` 内のすべての VM が同じプライベートネットワークに接続され、互いに簡単に通信できるようになります。

## マニフェストの例

```yaml
apiVersion: apps/v1
kind: Fleet
metadata:
  name: my-app-stack
  namespace: default
spec:
  networkClassName: app-private-net
  components:
    - name: frontend
      replicas: 2
      shipTemplate:
        metadata:
          labels:
            component: frontend
        spec:
          shipClassName: small
          image: "my-registry.local/frontend:v1"
    - name: backend
      replicas: 2
      shipTemplate:
        metadata:
          labels:
            component: backend
        spec:
          shipClassName: medium
          image: "my-registry.local/backend:v1"
```

## 更新の挙動

`Fleet` のコンポーネントテンプレートが更新されると、`Fleet` コントローラーは対応する `ReplicaSet` のテンプレートを直接更新することで **インプレース更新** を実行します。その後、`ReplicaSet` コントローラーが個々の `Ship` の更新を処理します。`Deployment` とは異なり、`Fleet` は現在、更新に `ReplicaSet` のローテーションを使用しません。

## ステータス

`FleetStatus` は以下を追跡します。
- `totalComponents`: `Fleet` で定義されているコンポーネントの総数。
- `readyComponents`: 希望するすべてのレプリカの準備ができているコンポーネントの数。
