# タスク: スケジューラへの RuntimeClass フィルタープラグインの追加

## 前提

タスク `01-proto-definition.md` および `03-ship-runtime-class-field.md` が完了済みであること。

## 概要

スケジューラに `RuntimeClassFit` フィルタープラグインを追加する。
Ship の `spec.runtime_class` が指定されている場合、ノードに対応する `RuntimeClass` が存在し、
かつその `RuntimeClass` が Ship の要件を満たしているかを確認してノードを絞り込む。

受け入れ条件で明示されているのは「`live_migration` フラグを参照し、ライブマイグレーション非対応のノードには
対応が必要な Ship をスケジュールしない」ことだが、将来のホットプラグ対応も見据えた設計にする。

## 実装内容

### 1. Cache への RuntimeClass の追加

`tugboat-scheduler/src/cache.rs` を更新する:

- `runtime_classes: Vec<RuntimeClass>` フィールドを追加
- `refresh()` で `Api::<RuntimeClass>::all(...)` を使って一覧取得
- `runtime_classes()` → `&[RuntimeClass]` と `find_runtime_class(name: &str)` アクセサを追加

### 2. SchedulingContext への RuntimeClass の追加

`tugboat-scheduler/src/framework/types.rs` の `SchedulingContext` を更新する:

- `all_runtime_classes: Vec<RuntimeClass>` フィールドを追加
- `find_runtime_class(name: &str) -> Option<&RuntimeClass>` メソッドを追加

### 3. scheduler.rs での SchedulingContext 構築の更新

`tugboat-scheduler/src/scheduler.rs` の `schedule_ship` 内で `SchedulingContext` を構築している箇所に
`all_runtime_classes: cache.runtime_classes().to_vec()` を追加する。

### 4. RuntimeClassFit プラグインの実装

`tugboat-scheduler/src/plugins/runtime_class_fit.rs` を新規作成する:

```rust
// Filter ロジックの概要:
// 1. Ship の spec.runtime_class が None または空文字列なら Accept（RuntimeClass 未指定は制約なし）
// 2. ノードの node_meta に runtime_class ラベル/アノテーション等でノードの RuntimeClass 名を特定する
//    ※ Node リソースに runtime_class フィールドがない場合は、ノード名と同名の RuntimeClass を探すか、
//    あるいは Node の labels/annotations に "tugboat.cloud/runtime-class" キーを使う設計とする
//    （設計判断を実装時に確認すること）
// 3. ノードに対応する RuntimeClass が存在しない → Reject
// 4. Ship が live_migration を必要とする条件の確認:
//    - ShipSpec.target_node_name が Some の場合（ライブマイグレーション進行中）、
//      RuntimeClass.spec.live_migration == true でなければ Reject
// 5. 上記をすべて通過したら Accept
```

**注意**: ノードと RuntimeClass の対応付け方法について、現在 `Node` リソースに `runtime_class` フィールドが
存在しないため、設計を決定してから実装すること。推奨案は Node の `labels` に
`"tugboat.cloud/runtime-class": "<runtimeclass-name>"` を設定する方式。

### 5. プラグインの登録

`tugboat-scheduler/src/plugins/mod.rs` に `pub mod runtime_class_fit;` を追加する。

`tugboat-scheduler/src/lib.rs` または `main.rs` でフレームワークに `RuntimeClassFit` プラグインを登録する。

## 自動テスト

`tugboat-scheduler/src/plugins/runtime_class_fit.rs` にユニットテストを追加する:

```rust
#[cfg(test)]
mod tests {
    // ケース1: runtime_class 未指定の Ship → どのノードでも Accept
    // ケース2: runtime_class 指定、ノードに対応する RuntimeClass あり・live_migration=true、
    //          マイグレーション進行中 Ship → Accept
    // ケース3: runtime_class 指定、ノードに対応する RuntimeClass あり・live_migration=false、
    //          マイグレーション進行中 Ship → Reject
    // ケース4: runtime_class 指定、ノードに対応する RuntimeClass が存在しない → Reject
    // ケース5: runtime_class 指定、通常 Ship（マイグレーションなし）→ Accept
}
```

## 確認コマンド

```bash
cargo build --package tugboat-scheduler
cargo test --package tugboat-scheduler
cargo clippy --package tugboat-scheduler
```
