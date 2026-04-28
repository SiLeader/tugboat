# Task 8: (オプション) 共有コードの抽出 — tugboat-runtime-common

## 目的

QEMU runtimeとCloud Hypervisor runtime間で重複しているハイパーバイザ非依存コードを共有クレートに抽出し、重複を解消する。

## 依存タスク

- Task 1〜7 全て完了後

## 前提条件

- Cloud Hypervisor runtimeが安定して動作していること
- このタスクはオプションであり、両ランタイムが安定した後のリファクタリングとして実施する

## 実装内容

### 1. 抽出対象モジュール

| モジュール | 行数 | 内容 |
|-----------|------|------|
| `config.rs` | ~28行 | JSON/ファイルからの設定読み込み |
| `validate.rs` の `validate_safe_id` | ~50行 | パストラバーサル防止バリデーション |
| `pre/mod.rs` | ~125行 | マウントNS入場、ネットワークNS作成、デーモン化、ユーザー切り替え |
| `utils/mod.rs` | ~73行 | FIFOベースのシグナル機構 |

合計: **~276行**

### 2. 作成するクレート

```
tugboat-runtime-common/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── config.rs
    ├── validate.rs
    ├── pre.rs      (namespace, daemonize, user switching)
    └── signal.rs   (FIFO-based signal mechanism)
```

### 3. 変更するファイル

- `Cargo.toml` (ワークスペース): `tugboat-runtime-common` を members に追加
- `tugboat-qemu-runtime/Cargo.toml`: `tugboat-runtime-common` を依存に追加
- `tugboat-cloud-hypervisor-runtime/Cargo.toml`: `tugboat-runtime-common` を依存に追加
- 両ランタイムの `lib.rs`: 共有モジュールの `mod` 宣言を削除し、`use tugboat_runtime_common::*` に置き換え
- 両ランタイムのインポートパスを更新

### 4. 残すもの（ランタイム固有）

QEMU runtime:
- `validate_qemu_option_value` — QEMU固有のバリデーション
- `cmd/qmp.rs` — QMP接続ユーティリティ
- `execute/vm/qemu/` — QEMU引数構築

Cloud Hypervisor runtime:
- `validate_ch_option_value` — Cloud Hypervisor固有のバリデーション（Task 3で追加した場合）
- `cmd/api.rs` — REST APIクライアント
- `execute/vm/cloud_hypervisor/` — Cloud Hypervisor引数構築

## 受け入れ条件

- [ ] `tugboat-runtime-common` クレートが作成されている
- [ ] 両ランタイムが `tugboat-runtime-common` に依存している
- [ ] 重複コードが削除されている
- [ ] `cargo build --workspace` が成功する
- [ ] `cargo test --workspace` がパスする
- [ ] `cargo clippy --workspace` がパスする
- [ ] 既存のQEMU runtimeの動作が変わらない

## テスト・確認方法

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace
cargo fmt --check
```

ユニットテスト:
- `tugboat-runtime-common` の各モジュールのテスト（validate.rs のテストを移動）
- 両ランタイムの既存テストが全てパスする
