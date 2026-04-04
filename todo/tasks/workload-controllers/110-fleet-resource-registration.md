# Fleet リソースの登録（manifests + バリデーター）

## 概要

`tugboat-resources` の `apply_resource!` / `apply_validators!` マクロを使って
Fleet を Tugboat のリソースシステムに登録する。

## 対象ファイル

- `tugboat-resources/src/manifests/mod.rs`

## 実装内容

### apply_resource! の追加

既存の `Deployment` や `ReplicaSet` と同じ場所に追加する：

```rust
apply_resource!(Fleet, "apps", "v1", "fleets", "fleet", namespaced);
```

### apply_validators! の追加

```rust
apply_validators!(Fleet, validators NameValidator);
```

### apps/v1 モジュールへの pub use

`tugboat-resources/src/manifests/apps/v1/mod.rs`（または相当するファイル）に
`Fleet` の再エクスポートが必要な場合は追加する。

## 完了条件

- `Fleet` が `tugboat_resources::manifests::apps::v1::Fleet` としてアクセスできる
- `Fleet::group()` → `"apps"`、`Fleet::version()` → `"v1"`、`Fleet::plural()` → `"fleets"` が返る
- `cargo build --release --package tugboat-resources` が通ること
- `cargo clippy` で警告がないこと
