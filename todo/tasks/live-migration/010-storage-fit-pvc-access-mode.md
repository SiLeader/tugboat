# StorageFit スケジューラフィルタ: PVC のアクセスモードもチェックする

## 概要

`tugboat-scheduler/src/plugins/storage_fit.rs` の `StorageFitFilter` は現状 PersistentVolume (PV) の
`access_modes` のみを検査している。一方、`tugboat-agent` のマイグレーション preflight
(`migration.rs` の `validate_storage_eligibility`) は PVC と PV の両方が `ReadWriteMany`
であることを要求している。

スケジューラ側も PVC のアクセスモードを確認し、RWX 未対応の PVC を参照する Ship を
RWX 非対応 PV のノードと同様に弾くよう統一する。

## 対象ファイル

- `tugboat-scheduler/src/plugins/storage_fit.rs`

## 修正内容

`StorageFitFilter::filter` 内で PV の `access_modes` を検査するのと同様に、
対応する PVC の `spec.access_modes` も `ReadWriteMany` を含むか確認する。
どちらか一方でも RWX を持たない場合は `FilterResult::Reject` を返す。

`SchedulingContext::ship_bound_persistent_volumes()` が PV を返しているため、
同様に PVC を返すヘルパーを活用するか、PVC → PV のペアで検査するよう実装する。

## 完了条件

- PVC が `ReadWriteMany` を持たない場合にフィルタが Reject を返す
- 既存テストが通ること
- PVC アクセスモードを検証する新しいユニットテストを追加すること
