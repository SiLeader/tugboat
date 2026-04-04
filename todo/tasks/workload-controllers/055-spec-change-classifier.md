# ShipTemplate 変更分類ユーティリティ

## 概要

Deployment コントローラーが ShipTemplate の変更内容を分析し、
**インプレース更新で対応可能か、ReplicaSet ローテーションが必要か**を判別する
ユーティリティを実装する。

この分類により Deployment は：
- インプレース可能な変更 → 既存 ReplicaSet のテンプレートを直接更新（Ship は再作成されない）
- 再作成が必須な変更 → 新しい ReplicaSet を作成してローテーション（従来方式）

を使い分けられるようになる。

## 対象ファイル

- `tugboat-controller-manager/src/change_classifier.rs`（新規作成）

## 設計

### 変更カテゴリ

エージェントの `spec_fingerprint` の内訳をもとに、フィールドを 3 カテゴリに分類する：

| カテゴリ | フィールド | 理由 |
|---|---|---|
| **InPlace（インプレース）** | `ship_class` | ホットプラグ or エージェント再作成で対応。コントローラーは Ship を殺さない |
| **InPlace** | `volumes`（ConfigMap/Secret） | エージェントがインプレースリフレッシュで対応 |
| **RequiresRotation（ローテーション必須）** | `image` | VM イメージの変更は再作成が必須 |
| **RequiresRotation** | `uefi` | ファームウェア設定は再作成が必須 |
| **RequiresRotation** | `network_class_ref` | NIC 構成の変更は現状再作成が必須 |
| **RequiresRotation** | `volume_claim_ref` | PVC バインディング変更は再作成が必須 |

> 将来 NIC ホットプラグが実装されたら `network_class_ref` を InPlace に移動できる。

### 分類結果の型

```rust
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TemplateChangeKind {
    /// 変更なし
    NoChange,
    /// 既存 ReplicaSet のテンプレートを更新するだけでよい。
    /// エージェントがホットプラグ / マイグレーション / 再作成を判断する。
    InPlace,
    /// 新しい ReplicaSet を作成して Ship をローテーションする必要がある。
    RequiresRotation,
}
```

### 分類関数

```rust
pub(crate) fn classify_template_change(
    old: &ShipTemplateSpec,
    new: &ShipTemplateSpec,
) -> TemplateChangeKind {
    if old == new {
        return TemplateChangeKind::NoChange;
    }

    let rotation_required =
        old.spec.image != new.spec.image
        || old.spec.uefi != new.spec.uefi
        || old.spec.network_class_ref != new.spec.network_class_ref
        || old.spec.volume_claim_ref != new.spec.volume_claim_ref;

    if rotation_required {
        TemplateChangeKind::RequiresRotation
    } else {
        TemplateChangeKind::InPlace
    }
}
```

### テスト

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn ship_class_change_is_in_place() { /* ship_class のみ変更 → InPlace */ }

    #[test]
    fn image_change_requires_rotation() { /* image 変更 → RequiresRotation */ }

    #[test]
    fn configmap_volume_change_is_in_place() { /* volumes の ConfigMap 変更 → InPlace */ }

    #[test]
    fn pvc_change_requires_rotation() { /* volume_claim_ref 変更 → RequiresRotation */ }

    #[test]
    fn mixed_change_requires_rotation() { /* ship_class + image 変更 → RequiresRotation */ }
}
```

## 完了条件

- `classify_template_change` が正しくカテゴリを返す
- 単体テストが全パターンをカバーしている
- `cargo clippy` および `cargo test` が通ること
