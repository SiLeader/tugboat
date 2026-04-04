# マイグレーション preflight: ターゲットノードのリソース容量チェックを追加

## 概要

`tugboat-agent/src/reconciler/ops/migration.rs` の `preflight_migration` は
現状、CNI 準備状態・アーキテクチャ一致・ストレージ RWX のみを確認している。

ターゲットノードに Ship を移動するための CPU/メモリの空き容量チェックが行われていない。
スケジューラの `ResourceFitFilter` は初回スケジューリング時に行うが、
マイグレーション時点では Ship はすでに source ノードに配置済みのため
スケジューラは介在しない。preflight でリソース不足のノードへの移行を拒否すべきである。

## 対象ファイル

- `tugboat-agent/src/reconciler/ops/migration.rs`
- `tugboat-agent/src/reconciler/` (リソース使用量計算ロジックの参照先)

## 実装内容

`ShipReconciler::preflight_migration` に以下を追加する:

1. ターゲットノードの `spec.resource` および `spec.overcommit` から allocatable なリソースを取得
2. ターゲットノード上で稼働中の全 Ship のリソース使用量を合算（`node_name` で Ship を検索）
3. マイグレーション対象 Ship の ShipClass から要求リソースを取得
4. `available = allocatable - used` が `requested` を下回る場合は `MigrationPreflight::Reject` を返す

スケジューラの `ResourceFitFilter` に実装済みのロジック（`tugboat-scheduler/src/plugins/resource_fit.rs`）
を参考にして実装する。

## 完了条件

- CPU または メモリが不足しているターゲットノードへの移行が preflight で拒否される
- 既存のユニットテストが通ること
- リソース不足ケースのユニットテストを追加すること
