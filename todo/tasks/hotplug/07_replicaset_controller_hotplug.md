# タスク 07: ReplicaSet コントローラのホットプラグ対応

## 前提タスク

- タスク 06 (change_classifier の Hotplug 分類) が完了していること

## 目的

`tugboat-controller-manager/src/replicaset.rs` を変更し、テンプレート変更が `Hotplug` 分類に
なった場合に新しい ReplicaSet を作成せずに既存の Ship を直接更新するフローを実装する。

## 実装内容

### `tugboat-controller-manager/src/replicaset.rs` の変更

現在の ReplicaSet 更新フロー（ローテーション）に加え、`TemplateChangeKind::Hotplug` の場合は
既存 Ships の spec を新しいテンプレートに従って直接 PATCH する。

#### 変更箇所のイメージ

```rust
// 既存の Ships に対してテンプレート変更を適用する部分
match classify_template_change(&old_template, &new_template, runtime_class.as_ref()) {
    TemplateChangeKind::NoChange => { /* nothing */ }
    TemplateChangeKind::InPlace => {
        // 既存: in-place 更新（ConfigMap/Secret ボリュームのみ）
    }
    TemplateChangeKind::Hotplug => {
        // 新規: 各 Ship の spec を新テンプレートで上書き PATCH する
        // Agent がホットプラグを実行する（コントローラは spec を書くだけ）
        for ship in owned_ships {
            patch_ship_spec(&client, ship, &new_template.spec).await?;
        }
    }
    TemplateChangeKind::RequiresRotation => {
        // 既存: ローテーション（新 ReplicaSet 作成）
    }
}
```

コントローラは Ship の spec を更新するだけでよい。実際のホットプラグ実行は Agent（タスク 05）が行う。

### RuntimeClass の取得

`classify_template_change` に RuntimeClass を渡すため、ReplicaSet が参照する RuntimeClass を
API 経由で取得するロジックを追加する。RuntimeClass が取得できない場合は `None` を渡し
従来のローテーション動作にフォールバックする。

### `tugboat-controller-manager/src/fleet.rs`、`deployment.rs` も同様に確認

`classify_template_change` の呼び出し箇所があれば同様に `runtime_class` を渡すよう更新する。

## テスト

コントローラのロジックは実際の API サーバーが必要なため、単体テストは困難。
以下を確認してテストとする:

- `cargo test --package tugboat-controller-manager` が通ること（既存テストの回帰確認）
- `cargo clippy --package tugboat-controller-manager` がエラーなく通ること
