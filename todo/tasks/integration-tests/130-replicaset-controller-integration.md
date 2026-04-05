# ReplicaSet コントローラー結合テスト

## 概要

ReplicaSet コントローラーが API サーバーと連携し、指定されたレプリカ数の Ship を
作成・維持するフローを検証する。

## 前提

- API サーバーとコントローラーマネージャーが起動していること

## テストシナリオ

### シナリオ 1: Ship の自動作成

1. Namespace を作成
2. `spec.replicas=3` の ReplicaSet を作成
3. コントローラーが Ship を 3 つ作成するまで待機（ポーリング）
4. Ship の一覧を取得し 3 つ存在することを確認
5. 各 Ship の `metadata.ownerReferences` に ReplicaSet が含まれること
6. 各 Ship のラベルが ReplicaSet の `spec.selector` と一致すること

### シナリオ 2: スケールアップ

1. `spec.replicas=2` の ReplicaSet を作成し Ship が 2 つ作られるのを待つ
2. ReplicaSet をパッチして `spec.replicas=5` に変更
3. Ship が 5 つになるまで待機
4. 新しい Ship の内容が template と一致することを確認

### シナリオ 3: スケールダウン

1. `spec.replicas=3` の ReplicaSet を作成し Ship が 3 つ作られるのを待つ
2. ReplicaSet をパッチして `spec.replicas=1` に変更
3. Ship が 1 つになるまで待機

### シナリオ 4: Ship 削除時の再作成

1. `spec.replicas=2` の ReplicaSet を作成し Ship が 2 つ作られるのを待つ
2. Ship の 1 つを手動で削除
3. コントローラーが新しい Ship を作成し、再び 2 つになることを確認

### シナリオ 5: ReplicaSet 削除

1. ReplicaSet を作成し Ship が作られるのを待つ
2. ReplicaSet を削除
3. 所有する Ship も削除されることを確認（カスケード削除の場合）

## 完了条件

- レプリカ数の維持（作成・スケールアップ・スケールダウン）がテストされていること
- Ship 削除時の自動復旧がテストされていること
- `cargo test` が通ること
