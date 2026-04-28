# Task 7: サンプル設定ファイルとドキュメント更新

## 目的

Cloud Hypervisor runtimeのサンプル設定ファイルを追加し、CLAUDE.mdのアーキテクチャセクションを更新する。

## 依存タスク

- Task 1（スキャフォールド）

## 実装内容

### 1. 作成するファイル

| ファイル | 内容 |
|---------|------|
| `sample-configs/runtime/cloud-hypervisor-config.toml` | Cloud Hypervisor runtime用サンプル設定 |

### 2. サンプル設定ファイル

```toml
[cloud_hypervisor]
executable = "/usr/bin/cloud-hypervisor"
disk_image_location = "/var/lib/tugboat-agent/images"

# Direct kernel boot (recommended)
[cloud_hypervisor.boot]
kernel = "/var/lib/tugboat-agent/vmlinux"
# initramfs = "/var/lib/tugboat-agent/initramfs.img"

# UEFI boot (alternative, uncomment below and comment kernel above)
# [cloud_hypervisor.boot]
# firmware = "/usr/share/OVMF/OVMF.fd"
```

### 3. CLAUDE.md 更新

コンポーネント依存グラフに追加:
```
tugboat-cloud-hypervisor-runtime (Cloud Hypervisor executor)
  ├─ tugboat-resources
  └─ tugboat-vm-runtime-interface
```

設定セクションにパス追加:
- `/etc/tugboat/runtime/cloud-hypervisor-config.toml`

### 4. エージェント設定例の追記

`sample-configs/agent/config.toml` がある場合、Cloud Hypervisorを使用する場合のランタイム設定例をコメントで追記:

```toml
# Cloud Hypervisor runtime (alternative to QEMU)
# [runtime]
# executable = "/usr/bin/tugboat-cloud-hypervisor-runtime"
# args = ["--config", "/etc/tugboat/runtime/cloud-hypervisor-config.toml"]
```

### 5. 参照ファイル

- `sample-configs/runtime/config.toml` (28行): QEMU版サンプル設定
- `CLAUDE.md`: アーキテクチャドキュメント

## 受け入れ条件

- [ ] `sample-configs/runtime/cloud-hypervisor-config.toml` が存在する
- [ ] サンプル設定が有効なTOMLである
- [ ] CLAUDE.md にCloud Hypervisor runtimeが記載されている
- [ ] 直接カーネルブートとUEFIブートの両方の設定例が含まれている

## テスト・確認方法

```bash
# TOML構文チェック（Rustコードでデシリアライズテストを書くか、手動確認）
cargo build --package tugboat-cloud-hypervisor-runtime
# 設定ファイルの読み込みテストは Task 1 のスキャフォールドテストに含める
```
