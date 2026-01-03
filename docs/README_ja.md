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
- etcdベースの宣言的クラスタ
    - apiserverはstatelessでetcdが唯一の状態
- 軽量なコントロールプレーン
    - apiserverとschedulerのみ
- QEMUを直接使用
    - libvirtを使わずQEMUを直接exec
- agent / runtimeの責務分離
    - Kubernetesのkubelect / runtimeと同じ思想
- OCIアーティファクトとしてのVMイメージ
    - Imagefile → build → registryにpush → Shipから参照
- CNIに対応予定
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
|    kubelect     |     agent      |

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
    architecture: x86_64
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
```

## Roadmap

- [x] tugboat-runtime
- [x] tugboat-resources (Resource definitions)
- [x] etcd wrapper for apiserver
- [ ] tugboat-apiserver
- [ ] tugboat-agent
- [ ] tugboat-scheduler
- [ ] CNI
- [ ] storage
- [ ] Fleet
- [ ] Secret / ConfigMap
- [ ] Live migration
- [ ] CRD

## Contributing

まだ初期段階のプロジェクトですが、ご協力いただける方は歓迎します。

## License

Apache License 2.0

[LICENSE](../LICENSE)を参照してください。
