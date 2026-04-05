# PersistentVolume と PersistentVolumeClaim のバインド結合テスト

## 概要

PersistentVolume（PV）と PersistentVolumeClaim（PVC）の作成およびステータス管理を
API レベルで検証する。ストレージリソースのライフサイクル全体をテストする。

## テストシナリオ

### シナリオ 1: PV の作成とステータス管理

1. StorageClass `standard` を作成
2. PersistentVolume `pv-1` を作成（StorageClass を指定）
3. 取得し spec を検証
4. ステータスを `Available` に更新
5. 再取得しステータスが反映されていることを確認

### シナリオ 2: PVC の作成とステータス管理

1. Namespace を作成
2. PersistentVolumeClaim `pvc-1` を作成（StorageClass、容量、アクセスモードを指定）
3. 取得し spec を検証
4. ステータスを更新（バインドされた PV 名を設定）
5. 再取得しステータスが反映されていることを確認

### シナリオ 3: PV の削除

1. PV を作成
2. 削除
3. 取得で 404 が返ることを確認

### シナリオ 4: PVC の削除

1. PVC を作成
2. 削除
3. 取得で 404 が返ることを確認

## 完了条件

- PV と PVC の CRUD およびステータス管理がテストされていること
- `cargo test` が通ること
