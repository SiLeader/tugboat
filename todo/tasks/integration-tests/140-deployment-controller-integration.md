# Deployment コントローラー結合テスト

## 概要

Deployment コントローラーが API サーバーと連携し、ReplicaSet の作成とローリングアップデートを
正しく管理するフローを検証する。

## 前提

- API サーバーとコントローラーマネージャーが起動していること

## テストシナリオ

### シナリオ 1: Deployment による ReplicaSet の自動作成

1. Namespace を作成
2. `spec.replicas=2` の Deployment を作成
3. ReplicaSet が自動作成されるまで待機
4. ReplicaSet の `spec.replicas` が 2 であることを確認
5. ReplicaSet の `metadata.ownerReferences` に Deployment が含まれること

### シナリオ 2: Deployment のスケーリング

1. Deployment を作成し ReplicaSet が作られるのを待つ
2. Deployment をパッチして `spec.replicas` を変更
3. ReplicaSet の `spec.replicas` が更新されることを確認

### シナリオ 3: ローリングアップデート

1. Deployment を作成し、Ship が起動するのを待つ
2. Deployment の `spec.template` を変更（イメージ変更等）
3. 新しい ReplicaSet が作成されることを確認
4. 古い ReplicaSet のレプリカ数が 0 になることを確認
5. 新しい ReplicaSet のレプリカ数が指定数になることを確認

### シナリオ 4: Deployment 削除

1. Deployment を作成し ReplicaSet と Ship が作られるのを待つ
2. Deployment を削除
3. 関連する ReplicaSet も削除されることを確認

## 完了条件

- Deployment による ReplicaSet 管理がテストされていること
- ローリングアップデートのフローがテストされていること
- `cargo test` が通ること
