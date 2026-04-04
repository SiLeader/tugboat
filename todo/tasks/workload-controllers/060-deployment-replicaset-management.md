# Deployment: ReplicaSet の作成・更新（インプレース更新パス対応）

## 概要

`DeploymentReconciler::reconcile_applied` に、Deployment が管理する ReplicaSet を
作成・更新するロジックを実装する。

**重要な設計方針**: Tugboat は VM を直接管理するため、Kubernetes とは異なり
ShipTemplate の変更内容によって 2 つの更新パスを使い分ける：

1. **インプレース更新パス**: `ship_class`（CPU/メモリ）や ConfigMap/Secret の変更など、
   エージェントがホットプラグ・マイグレーション・インプレースリフレッシュで対応できる変更。
   既存 ReplicaSet のテンプレートを直接更新する（ReplicaSet コントローラーが Ship を
   1 台ずつ更新する）。
2. **ローテーションパス**: VM イメージや UEFI 設定など、再作成が避けられない変更。
   新しい ReplicaSet を作成し、古い ReplicaSet をスケールダウンする（従来の方式）。

## 対象ファイル

- `tugboat-controller-manager/src/deployment.rs`

## 実装内容

### 変更パスの決定

`change_classifier.rs`（055 タスク）の `classify_template_change` を使う：

```rust
use crate::change_classifier::{classify_template_change, TemplateChangeKind};

let change_kind = if let Some(active_rs) = &active_rs {
    classify_template_change(&active_rs.spec.ship_template, &dep.spec.ship_template)
} else {
    TemplateChangeKind::RequiresRotation // RS がない場合は新規作成
};
```

### パス 1: インプレース更新（InPlace）

既存の ReplicaSet のテンプレートだけを更新する。Ship の実際の更新は
ReplicaSet コントローラー（025 タスク）が 1 台ずつ行う：

```rust
TemplateChangeKind::InPlace => {
    let mut updated_rs = active_rs.clone();
    updated_rs.spec.ship_template = dep.spec.ship_template.clone();
    if dep.spec.replicas != updated_rs.spec.replicas {
        updated_rs.spec.replicas = dep.spec.replicas;
    }
    rs_api.replace(active_rs.name(), updated_rs).await?;
}
```

### パス 2: ローテーション（RequiresRotation）

テンプレートハッシュで新旧 ReplicaSet を識別し、新 RS 作成 → 古い RS スケールダウン。

#### テンプレートハッシュ

`ship_template` を SHA-256 ハッシュして `ship-template-hash` ラベルとして
ReplicaSet に付与する：

```rust
fn template_hash(template: &ShipTemplateSpec) -> String {
    use sha2::Digest;
    let json = serde_json::to_string(template).unwrap_or_default();
    let hash = sha2::Sha256::digest(json.as_bytes());
    format!("{:x}", &hash[..4]) // 短縮ハッシュ（8 文字）
}
```

#### 新 ReplicaSet 作成

```rust
TemplateChangeKind::RequiresRotation => {
    // アクティブ RS がないか、ハッシュが異なる場合に新 RS を作成
    let hash = template_hash(&dep.spec.ship_template);
    let rs = ReplicaSet {
        spec: ReplicaSetSpec {
            replicas: dep.spec.replicas,
            selector: {
                let mut s = dep.spec.selector.clone();
                s.insert("ship-template-hash".to_string(), hash);
                s
            },
            ship_template: dep.spec.ship_template.clone(),
        },
        // ownerReference: Deployment
        ..
    };
    rs_api.create(rs).await?;
    // 古い RS は後続タスク（070/080）の戦略に従ってスケールダウン
}
```

### ownerReference

```rust
fn owner_reference_for_deployment(dep: &Deployment) -> OwnerReference {
    OwnerReference {
        api_version: "apps/v1".to_string(),
        kind: "Deployment".to_string(),
        name: dep.name().to_string(),
        uid: dep.uid().unwrap_or_default().to_string(),
        controller: Some(true),
    }
}
```

## 完了条件

- `ship_class` のみ変更した場合: 新しい ReplicaSet は作成されず、既存 RS のテンプレートが更新される
- `image` を変更した場合: 新しい ReplicaSet が作成される
- `ship_class` と `image` を同時に変更した場合: ローテーションパスが選ばれる
- `cargo clippy` が通ること
