# Tugboat

Tugboatは、KubernetesのようにVMのオーケストレーションを行うシステムです。

## Introduction

Tugboatは、Kubernetesのような機能を提供するVMのオーケストレーションツールです。
KubeVirtのような重量感を軽減し、OpenStackのような複雑さを排除します。

Tugboatはetcdとtugboat-apiserver、tugboat-agent、tugboat-runtime、tugboat-schedulerという最小構成でVMをKubernetesのように管理することを目指しています。

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
- CRDに対応予定
- HA設計
    - apiserverは水平スケール可能
    - schedulerはLeaseによる調停

## Architecture overview

![architecture overview](./images/tugboat-structure.svg)

### Kubernetesリソースとの対応

|   Kubernetes    |    Tugboat     |
|:---------------:|:--------------:|
|       Pod       |      Ship      |
|   Deployment    |   Fleet (予定)   |
|      Node       |      Node      |
| Container image | VM image (OCI) |
|   Dockerfile    |   Imagefile    |
|     kubelet     |     agent      |

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
  image: example.com/vm-images/ubuntu:24.04
  shipClass: lightweight
  volumeClaimRef:
    - name: data-disk
```

CSI volume を使う場合は、`volumeClaimRef` で同一 namespace の `PersistentVolumeClaim` を参照します。

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

`Filesystem` volume は guest へ 9p share として公開され、mount tag には `volumeClaimRef[].name` が使われます。

control plane 側では、`PersistentVolume`、`PersistentVolumeClaim`、`StorageClass` の API に加えて、`tugboat-controller-manager` による CSI の動的プロビジョニング、管理対象 PV の cleanup、容量指定付きの provision/expand、`Filesystem` claim、CSI secret / `fsType` の引き回しまで実装済みです。node 側も controller publish context、`NodeExpandVolume`、`NodeGetVolumeStats` による CSI health / usage の PV/PVC condition 反映まで対応しました。残る大きな課題は、scheduler の storage 制約考慮、永続 state 以上の recovery、snapshot / clone 系ワークフローです。

## Roadmap

- [x] tugboat-runtime
- [x] tugboat-resources (リソース定義)
- [x] tugboat-resource-store (apiserver向けのetcdラッパー)
- [x] tugboat-apiserver
- [x] tugboat-client
- [x] tugboat-cli build (ImagefileからVMイメージのビルド)
- [x] fieldSelectorとlabelSelector
- [x] tugboat-scheduler
- [ ] tugboat-agent (← イマココ)
    - [x] Nodeリソースの自動登録
    - [x] Ship Addedイベントのreconcile
    - [x] ネットワーク (CNI, NetworkClass / ClusterNetworkClass)
    - [x] Ship Modifiedイベントのreconcile
    - [x] Ship Deletedイベントのreconcile
    - [ ] ストレージ (CSI の provision / publish / stage / expand までは実装済み。topology-aware scheduling と snapshot 系は今後の課題)
- [x] Secret
- [ ] tugboat-controller-manager
    - [x] CSIの動的プロビジョニング
    - [x] CSI管理下PVのcleanup
    - [ ] ReplicaSet (Shipの規定数維持)
    - [ ] Deployment (同形式のShipのデプロイ)
    - [ ] Fleet
- [ ] ConfigMap
- [ ] Live migration
- [ ] RBAC / ServiceAccount
- [ ] CRD

## Contributing

まだ初期段階のプロジェクトですが、ご協力いただける方は歓迎します。

## License

Apache License 2.0

[LICENSE](../LICENSE)を参照してください。
