# tugboat-cloud-hypervisor-runtime 実装タスク

## タスク一覧

| # | タスク | 依存 | 概要 |
|---|--------|------|------|
| 1 | [scaffold](tasks/task-01-scaffold.md) | - | クレート構造、CLI、エラー型、共有モジュールコピー |
| 2 | [api-client](tasks/task-02-api-client.md) | 1 | Cloud Hypervisor REST APIクライアント (Unix socket HTTP) |
| 3 | [execute/create/start/run](tasks/task-03-execute-create-start-run.md) | 1 | VM起動、CLI引数構築、2フェーズライフサイクル |
| 4 | [stop/status](tasks/task-04-stop-status.md) | 1, 2 | VM停止・状態取得 |
| 5 | [hotplug](tasks/task-05-hotplug.md) | 1, 2 | CPU/メモリ/NIC/ボリュームホットプラグ（ロールバック付き） |
| 6 | [migration](tasks/task-06-migration.md) | 1, 2 | ライブマイグレーション（開始/キャンセル/状態取得） |
| 7 | [config/docs](tasks/task-07-config-docs.md) | 1 | サンプル設定ファイル、ドキュメント更新 |
| 8 | [shared-crate](tasks/task-08-shared-crate.md) | 1-7 | (オプション) 共有コード抽出 |

## 依存グラフ

```
Task 1 (scaffold)
  ├──> Task 2 (API client) ──> Task 4 (stop/status)
  │                        ──> Task 5 (hotplug)
  │                        ──> Task 6 (migration)
  ├──> Task 3 (execute/create/start/run)
  └──> Task 7 (config/docs)

Task 8 (optional) -- after all
```

## 並列実行可能なタスク

- Task 1 完了後: Task 2, 3, 7 を並列実行可能
- Task 2 完了後: Task 4, 5, 6 を並列実行可能
