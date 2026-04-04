# Fleet コントローラのマイグレーション中 Ship 保護

## 概要

Fleet（Deployment 相当）コントローラがローリングアップデートを実行する際、
ライブマイグレーション中の Ship を削除・置換すると競合が発生する可能性がある。

マイグレーション中（`status.migration.phase` が `Pending` / `Ready` / `Migrating`）の Ship は
Fleet からの更新・削除操作の対象から除外し、マイグレーション完了後に再評価するよう保護する。

## 対象ファイル

- Fleet / ReplicaSet 調停ロジック（`tugboat-scheduler` または専用コントローラのファイル）
- 現在の実装場所を確認してから対象ファイルを特定すること

## 実装内容

Fleet / ReplicaSet の reconcile ループで Ship を列挙する際、以下のフィルタを追加する:

```rust
fn is_migrating(ship: &Ship) -> bool {
    ship.status
        .as_ref()
        .and_then(|s| s.migration.as_ref())
        .map(|m| {
            matches!(
                m.phase.as_str(),
                "Pending" | "Ready" | "Migrating"
            )
        })
        .unwrap_or(false)
}
```

- `is_migrating` な Ship は「削除候補」に含めない
- `is_migrating` な Ship は「更新候補」に含めない（ローリングアップデート対象外）
- ただし Fleet の desired replica 数の計算時には migrating Ship も稼働中としてカウントする

## 完了条件

- マイグレーション中の Ship が Fleet のローリングアップデートで削除・置換されないこと
- マイグレーション完了後に Ship が通常の調停対象に戻ること
- `cargo test` が通ること
