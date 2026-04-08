# Tugboat

[English](./README.md)

Tugboatは、KubernetesのようにVMのオーケストレーションを行うシステムです。

## Introduction

Tugboatは、Kubernetesのような機能を提供するVMのオーケストレーションツールです。
KubeVirtのような重量感を軽減し、OpenStackのような複雑さを排除します。

Tugboatはetcdとtugboat-apiserver、tugboat-agent、tugboat-runtime、tugboat-scheduler、tugboat-controller-managerという最小構成でVMをKubernetesのように管理することを目指しています。

## Why Tugboat?

既存のVMオーケーストレーションには以下のような課題があります。

- KubeVirt
    - Kubernetesの上にVMを乗せるため、二重構造で重い
    - libvirtやCRDが複雑
- OpenStack
    - コンポーネントが多すぎて個人や小規模には扱えない
    - 学習コストが非常に高い
    - 運用が難しい

Tugboatはこれらの問題を解決するために生まれました。

- Kubernetesの思想を継承
- Kubernetesそのものは使わない
- VMに最適化した軽量な設計
- 個人でも動かせる単純さ
- 大規模化にも耐えられる構造

## Key Features

- Kubernetes互換のAPIマニフェスト
    - TypeMeta / ObjectMetaなどKubernetesと同じ構造
    - `kubectl`がそのまま利用可能
- etcdベースの宣言的クラスタ
    - apiserverはstatelessでetcdが唯一の状態
- 軽量なコントロールプレーン
    - apiserverとschedulerのみ
- QEMUを直接使用
    - libvirtを使わずQEMUを直接exec
- agent / runtimeの責務分離
    - Kubernetesのkubelet / runtimeと同じ思想
- OCIアーティファクトとしてのVMイメージ
    - Imagefile → build → registryにpush → Shipから参照
- CNIに対応
    - `NetworkClass` / `ClusterNetworkClass`によるネットワーク設定
    - agent が `Node.status.cniPlugins` に plugin readiness を公開
    - scheduler が `NetworkFit` で node を絞り込み
- CRDに対応予定
- HA設計
    - apiserverは水平スケール可能
    - schedulerはLeaseによる調停

## Architecture overview

![architecture overview](./docs/images/tugboat-structure.svg)

### Kubernetesリソースとの対応

|   Kubernetes    |    Tugboat     |
|:---------------:|:--------------:|
|       Pod       |      Ship      |
|   ReplicaSet    |   ReplicaSet   |
|   Deployment    |   Deployment   |
|      Node       |      Node      |
| Container image | VM image (OCI) |
|   Dockerfile    |   Imagefile    |
|     kubelet     |     agent      |
|  RuntimeClass   |  RuntimeClass  |

> **補足:** `Fleet` は複数の Ship タイプがプライベートネットワークを共有するグループを表す Tugboat 独自のリソースで、Kubernetes に直接対応するものはありません。

## Manifest examples

### ShipClass

VMのマシンタイプを定義するリソースです。
クラスタリソースです。

```yaml
apiVersion: v1
kind: ShipClass
metadata:
  name: lightweight
spec:
  cpu:
    architecture: x64
    cores: 2
  memory:
    size: 4Gi
```

### Ship

VMインスタンスを定義するリソースです。
Namespacedリソースです。

```yaml
apiVersion: v1
kind: Ship
metadata:
  namespace: default
  name: ship
spec:
  image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
  shipClass: lightweight
  volumes:
    - name: data-disk
      persistentVolumeClaim:
        claimName: data-disk
    - name: app-config
      configMap:
        name: app-config
    - name: app-secret
      secret:
        secretName: app-secret
```

### Fleet

複数の Ship タイプがプライベートネットワークを共有するグループを定義するリソースです。
Namespaced リソースです（`apps/v1`）。

```yaml
apiVersion: apps/v1
kind: Fleet
metadata:
  namespace: default
  name: my-fleet
spec:
  networkClassName: my-network-class
  components:
    - name: frontend
      replicas: 2
      shipTemplate:
        metadata:
          labels:
            role: frontend
        spec:
          image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
          shipClass: lightweight
    - name: backend
      replicas: 3
      shipTemplate:
        metadata:
          labels:
            role: backend
        spec:
          image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
          shipClass: lightweight
```

### Deployment

同一構成の Ship の集合をローリングアップデート付きで管理します。
Namespaced リソースです（`apps/v1`）。

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  namespace: default
  name: my-deployment
spec:
  replicas: 3
  selector:
    app: my-app
  shipTemplate:
    metadata:
      labels:
        app: my-app
    spec:
      image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
      shipClass: lightweight
```

### ReplicaSet

レプリカ Ship の安定したセットを維持します。
Namespaced リソースです（`apps/v1`）。

```yaml
apiVersion: apps/v1
kind: ReplicaSet
metadata:
  namespace: default
  name: my-replicaset
spec:
  replicas: 2
  selector:
    app: my-app
  shipTemplate:
    metadata:
      labels:
        app: my-app
    spec:
      image: ghcr.io/sileader/tugboat-vm-images/ubuntu:24.04
      shipClass: lightweight
```

### RuntimeClass

ノード上のVMランタイムが持つケーパビリティ（ライブマイグレーション対応、ホットプラグ対応）を宣言するリソースです。
クラスタリソースです。

```yaml
apiVersion: v1
kind: RuntimeClass
metadata:
  name: standard
spec:
  liveMigration: true
  hotplug:
    cpu:
      add: true
      remove: false
    memory:
      add: true
      remove: false
    nic:
      add: true
      remove: true
    storage:
      add: true
      remove: true
```

Ship は `spec.runtimeClass` フィールドで RuntimeClass 名を参照できます。
スケジューラの `RuntimeClassFit` プラグインが、Ship の要件（ライブマイグレーション対応など）を満たす RuntimeClass を持つノードにのみスケジューリングします。

### CSI サポート

CSI volume は `spec.volumes[].persistentVolumeClaim` で同一 namespace の
`PersistentVolumeClaim` を参照できます。後方互換のため、従来の
`volumeClaimRef` も引き続き受け付けます。

現時点の node-side CSI サポート範囲:

- [x] `Block` volumeMode
- [x] `Filesystem` volumeMode
- [x] `NodeStageVolume` / `NodeUnstageVolume` を要求する driver
- [x] `nodePublishSecretRef` / `nodeStageSecretRef`
- [x] agent restart 後の publish state からの復旧
- [x] 明示的な `fs_type` と `volume_attributes`
- [x] `NodeExpandVolume` / volume expansion
- [x] controller publish context を必須とする driver
- [x] `NodeGetVolumeStats` による CSI volume の health / usage を PV/PVC condition へ反映

`Filesystem` volume は guest へ 9p share として公開され、mount tag には
Ship volume の名前が使われます。そのため、CSI の `Filesystem` claim だけでなく
`ConfigMap` / `Secret` の projected volume も同じ経路で guest へ渡されます。

control plane 側では、`PersistentVolume`、`PersistentVolumeClaim`、`StorageClass` の API に加えて、
`tugboat-controller-manager` による CSI の動的プロビジョニング、管理対象 PV の cleanup、容量指定付きの provision/expand、
`Filesystem` claim、CSI secret / `fsType` の引き回しまで実装済みです。node 側も controller publish context、稼働中の Ship
を停止させない live `NodeExpandVolume`、`NodeGetVolumeStats` による CSI health / usage の PV/PVC condition
反映まで対応しました。残る大きな課題は、scheduler の storage 制約考慮、永続 state 以上の recovery、snapshot / clone
系ワークフローです。

### CNI status と Flannel 検証

現在の Tugboat は、agent が観測した CNI readiness を `Node.status.cniPlugins`
に公開し、`tugboat-controller-manager` が
`NetworkClass.status.readyNodes` /
`ClusterNetworkClass.status.readyNodes` を更新します。scheduler の
`NetworkFit` filter は、この status を見て、Ship が要求する
`NetworkClass` / `ClusterNetworkClass` に必要な plugin を持たない node を除外します。

現状の前提は、Flannel 自体は Tugboat の外でインストール・管理されることです。
手動で multi-node Flannel を確認する場合は、次の流れを想定しています。

1. 各 node の CNI bin directory に `bridge`、`loopback`、`flannel`、
   および port mapping を使う場合は `portmap` を配置する。
2. 外部管理の Flannel を起動し、各 node に既定の runtime state
   （既定では `/run/flannel/subnet.env` と `/run/flannel`）を用意する。
3. 各 node で `tugboat-agent` を起動し、`kubectl get node -o yaml` で
   `status.cniPlugins` に readiness が反映されていることを確認する。
4. `cniPlugin: flannel` を使う `ClusterNetworkClass` または `NetworkClass`
   を apply し、`status.readyNodes` に probe を通過した node が現れることを確認する。
5. その network class を参照する Ship を作成し、準備済み node にだけ
   schedule されることを確認してから、node 間疎通を検証する。

## Roadmap

- [x] tugboat-runtime
- [x] tugboat-resources (リソース定義)
- [x] tugboat-resource-store (apiserver向けのetcdラッパー)
- [x] tugboat-apiserver
- [x] tugboat-client
- [x] tugboat-cli build (ImagefileからVMイメージのビルド)
- [x] fieldSelectorとlabelSelector
- [x] tugboat-scheduler
- [x] tugboat-agent
    - [x] Nodeリソースの自動登録
    - [x] Ship Addedイベントのreconcile
    - [x] ネットワーク (CNI, NetworkClass / ClusterNetworkClass)
    - [x] Ship Modifiedイベントのreconcile
    - [x] Ship Deletedイベントのreconcile
    - [x] ストレージ (CSI publish/stage, controller publish context, および live expansion)
    - [ ] Topology-aware scheduling と snapshot 系ワークフロー
- [x] Secret
- [x] tugboat-controller-manager
    - [x] CSIの動的プロビジョニング
    - [x] CSI管理下PVのcleanup
    - [x] ReplicaSet コントローラ (Shipの規定数維持)
    - [x] Deployment コントローラ (ReplicaSetのローリングアップデート管理)
    - [x] Fleet コントローラ
- [x] Fleet リソース定義とAPI (`apps/v1`)
- [x] ReplicaSet リソース定義とAPI (`apps/v1`)
- [x] Deployment リソース定義とAPI (`apps/v1`)
- [x] ConfigMap
- [x] ライブマイグレーション
    - [x] `target_node_name` によるマイグレーションのトリガー
    - [x] マイグレーションのステートマシン (Pending, Ready, Migrating, Completed, Failed)
    - [x] Ship のステータス・条件へのマイグレーション状態の反映
    - [x] 事前互換性チェック (CPU、共有ストレージ適格性、ターゲットネットワーク対応)
    - [x] マイグレーション失敗時の確実な復旧と明示的なエラー報告
    - [x] ノード間でブリッジ/インターフェース/MACを固定することによるゲスト/ネットワーク継続性
    - [x] タイムアウト検出と自動QEMUキャンセル (Pending: 2分、Migrating: 30分)
    - [x] ShipClassごとのQEMUマイグレーションパラメータ設定 (帯域幅、ダウンタイム、xbzrleキャッシュ、ポストコピー)
    - [x] スケジューラの StorageFit プラグインによる非 RWX ボリュームを持つ Ship のノード除外
- [x] RuntimeClass
    - [x] リソース定義とAPI (`core/v1`)
    - [x] Ship の `spec.runtimeClass` フィールド
    - [x] スケジューラの `RuntimeClassFit` プラグイン（ライブマイグレーション対応チェック）
    - [x] RuntimeClass フラグによるホットプラグ操作の制御
- [ ] RBAC / ServiceAccount
- [ ] CRD

## Contributing

まだ初期段階のプロジェクトですが、ご協力いただける方は歓迎します。

## License

Apache License 2.0

[LICENSE](./LICENSE)を参照してください。
