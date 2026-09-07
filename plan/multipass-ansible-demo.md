# Multipass + Ansible による Tugboat デモ環境の実装計画

作成日: 2026-09-05。これは実装前の計画であり、記載する新規コマンド・ファイルは未実装。

## 1. 目的と初期スコープ

Canonical Multipass で Ubuntu VM を複数作成し、既存の Ansible インストーラーで Tugboat クラスタを構築する。利用者が環境作成、Ship の起動、異なるワーカー上の Ship 間通信、停止・再開、削除までを少数のコマンドで体験できるようにする。

初期版は Linux x86_64 ホスト、Ubuntu 24.04 ゲスト、QEMU/KVM、コントロールプレーン 1 台 + ワーカー 2 台を標準とする。ワーカー台数は 1 台以上で変更可能とし、2 台未満ではノード間通信の検証は対象外と明示する。Ansible はホストで実行する。

Multipass VM は Tugboat のノードであり、その内側で Tugboat が Ship VM を動かす二重の仮想化になる。Multipass 自体の起動成功だけではデモ成立としない。macOS、Windows、ARM、Cloud Hypervisor、HA、ライブマイグレーション、共有ストレージは後続拡張とする。対応範囲を狭める理由は、現行 QEMU 設定が x86_64/q35/OVMF と KVM 有効を前提としているため。

## 2. 既存実装と再利用方針

| 確認対象 | 現状と計画への反映 |
| --- | --- |
| `installer/ansible/site.yml` | control plane → worker → CSI の順に導入する。既存の入口をそのまま呼ぶ |
| `installer/ansible/roles/common/tasks/main.yml` | 主に変数の検証。ソース・Rust・ビルド成果物の準備は新規の準備 playbook が担当 |
| `installer/ansible/roles/tugboat_worker/` | control plane から CA、agent token、Flannel 用 etcd クライアント証明書・鍵を配布済み。重複実装しない |
| `installer/ansible/roles/tugboat_bootstrap/` | PKI/RBAC、ClusterNetworkClass、Flannel etcd 設定を作成済み |
| `installer/ansible/group_vars/all.yml` | secure=true、flannel=static が既定。デモ側で VXLAN と実 IP を指定する |
| `installer/ansible/roles/*/defaults/main.yml` | ビルド対象ディレクトリは VM 内に存在する必要がある。ホストのパスを渡すだけでは動かない |
| `installer/systemd/configs/runtime-qemu.config.toml.tpl` | `/usr/bin/qemu-system-x86_64`、KVM 有効、OVMF を指定。ネスト KVM の実動確認が必要 |
| `installer/ansible/ansible.cfg` | 相対 inventory/roles と host key checking 無効の設定がある。デモ専用 SSH 設定で上書きする |
| `installer/ansible/test/` | Docker 上の既存シナリオを維持し、Multipass の実 VM 検証を追加する |

Tugboat の導入処理は Ansible に集約する。ホストのスクリプトは Multipass 操作、接続情報・状態管理、Ansible 呼び出しを担い、systemd インストーラーを直接呼ばない。既存 role は `playbook_dir` に依存するため、新ディレクトリから安易に import せず、`installer/ansible` を作業ディレクトリとして絶対パスの inventory と vars を渡す。

## 3. 構成と前提条件

| VM | 役割 | 初期割当案 |
| --- | --- | --- |
| `<cluster>-cp-1` | etcd、API server、scheduler、controller manager。初回ビルドも実施 | 4 vCPU / 6 GiB / 40 GiB |
| `<cluster>-worker-1` | agent、QEMU、Flannel、Ship | 2 vCPU / 4 GiB / 30 GiB |
| `<cluster>-worker-2` | agent、QEMU、Flannel、Ship | 2 vCPU / 4 GiB / 30 GiB |

割当値は実測前の仮値。ホストにはゲスト合計 14 GiB に加えて OS 用メモリとビルド・イメージ転送用ディスクが必要。CPU、メモリ、ディスク、ビルド並列度を設定可能にし、実測後に README の推奨値を確定する。

前提ツールは Multipass、Python 3、Ansible Core 2.16 以上、OpenSSH、Git。Multipass のインストールやドライバー変更、ホストの KVM 設定変更は自動化せず、事前条件として案内する。Rust の準備はビルド VM 内の Ansible task が担当する。

標準の Multipass ネットワークを使い、ホストから全 VM への SSH/API 到達と VM 相互の到達を検査する。ブリッジ作成や物理 LAN 公開を必須にしない。対応 Multipass バージョン・ドライバーは最初の実機検証で固定し、`doctor` で記録する。

## 4. 追加するファイル

```text
installer/multipass/
  README.md                       # 手順、前提、構成図、復旧方法
  demo.py                         # CLI、Multipass 操作、状態管理
  config.example.json             # 台数・資源・ネットワーク・入力成果物
  cloud-init.yaml.j2              # ubuntu ユーザー維持、専用公開鍵、Python
  ansible/
    prepare.yml                   # ソース転送、ビルド、成果物配布
    verify.yml                    # サービス、API、CNI、KVM の確認
    demo.yml                      # デモ用リソース・イメージ準備と検証
    collect.yml                   # 診断情報収集
  manifests/                      # ShipClass、Ship、必要なネットワーク等
  test/
    test_demo.py                  # CLI/状態/障害処理のテスト
    run-smoke.sh                  # 実 VM の opt-in テスト
```

生成物は `.cache/multipass/<cluster>/` 配下に集約する。既存 `.gitignore` の `/.cache` を利用する。`state.json`、inventory、vars、専用 SSH 鍵・known_hosts、kubeconfig、成果物 manifest、ログを配置し、ディレクトリは 0700、秘密情報は 0600 とする。

Python は CLI の引数解析、JSON、subprocess、状態の原子的更新に標準ライブラリを使う。inventory/vars も Ansible が読める JSON として出力し、シェルの文字列展開による YAML 生成や `shell=True` を避ける。

## 5. 利用者向けコマンド案

リポジトリルートでの想定操作:

```bash
python3 installer/multipass/demo.py doctor
python3 installer/multipass/demo.py up --cluster trial --workers 2
python3 installer/multipass/demo.py status --cluster trial
python3 installer/multipass/demo.py demo --cluster trial
python3 installer/multipass/demo.py kubeconfig --cluster trial
python3 installer/multipass/demo.py stop --cluster trial
python3 installer/multipass/demo.py start --cluster trial
python3 installer/multipass/demo.py collect --cluster trial
python3 installer/multipass/demo.py destroy --cluster trial --yes
```

- `up`: 作成・準備・インストール・クラスタ検証まで実施する。途中失敗後は再実行できる。
- `provision`: 既存 VM に準備・インストールを再適用する。ソース変更は明示的な更新指定で取り込む。
- `demo`: サンプル Ship を作り、起動と通信を確認する。再実行で無関係なリソースを増やさない。
- `status`: VM 状態、API 到達、期待ワーカー数、CNI readiness、Ship 状態を表示する。
- `kubeconfig`: 専用ファイルのパスと使用例を表示し、既存の `~/.kube/config` を変更しない。
- `start`: 起動後に IP と接続・証明書情報を再評価し、必要な再設定を行って検証する。
- `destroy`: 所有を確認できる対象 VM と当該クラスタの秘密情報を削除する。対象一覧を出し、対話確認または `--yes` を要求する。

設定は CLI > 指定設定ファイル > 既定値の順とする。クラスタ名は Multipass 命名規則を満たす形式に限定し、台数・資源量・CIDR の不正値は VM 作成前に検出する。

## 6. 構築フロー

### 6.1 事前検査と VM 作成

1. ツール、Multipass daemon、ホスト arch、空き容量、名前衝突を検査する。
2. クラスタ UUID と専用 SSH 鍵を生成し、クラスタ単位で排他ロックを取得する。
3. cloud-init には `ubuntu` ユーザーを維持した公開鍵設定と Ansible 接続に必要な最小限の準備を書く。Tugboat のインストールは含めない。
4. `multipass launch 24.04 --name ... --cpus ... --memory ... --disk ... --cloud-init ...` を実行する。成功した VM を逐次 state に記録し、VM 内にも所有 UUID を保存する。
5. 起動と cloud-init 完了を期限付きで待つ。launch がタイムアウトしても VM が残る場合を考慮し、同名 VM を直ちに再作成しない。
6. Multipass の JSON 出力から VM 情報を取得する。複数 IP がある場合は管理ネットワークと到達性から選び、CNI bridge/VXLAN の IP を除外する。
7. 初期 SSH host key を `multipass exec` 経由で取得し、専用 known_hosts に固定する。Ansible は host key checking 有効、専用鍵、`become: true` で接続する。
8. 全ワーカーで `/dev/kvm` の存在・実行ユーザーのアクセス・最小 QEMU/KVM 起動を検査する。CPU flag の存在だけを合格条件にしない。

KVM 不可なら理由と前提条件を表示して中止する。TCG は自動フォールバックさせない。現行 runtime は KVM 無効の設定を持つが、デモでの CPU モデル、性能、起動時間を検証してから別モードとして検討する。

### 6.2 ビルドと配布

初期版の標準は「control plane VM で一度ビルドし、各 VM に prebuilt として配布」。公開リリースバイナリの存在やホストの Rust 環境を前提としない。

- `prepare.yml` がローカルの指定 Git commit のソースアーカイブを転送する。未コミット変更は既定で含めず、明示オプションでスナップショット化し digest を記録する。
- ビルド依存、Rust toolchain、必要なサブモジュール等を調査して準備し、`Cargo.lock` に従い `--locked` で必要パッケージを release build する。ツールチェーン・外部取得物のバージョンと checksum を記録する。
- API server、scheduler、controller manager、agent、QEMU runtime、デモに必要な CLI をビルドする。正確なバイナリ一覧は installer の参照と照合して確定する。
- 成果物をホストのキャッシュ経由で各 VM の `/opt/tugboat/bin` に配布する。`tugboat_build_mode: prebuilt` と `tugboat_prebuilt_bin_dir: /opt/tugboat/bin` を渡す。
- 外部の prebuilt ディレクトリ指定も用意し、Linux/arch、実行可能性、共有ライブラリ、必須バイナリ、checksum を検査する。
- キャッシュキーにはソース digest、target arch、toolchain、ビルドオプションを含める。キャッシュヒット時も成果物を検証する。
- 既存 role の install marker は主に引数・ファイル存在等に基づくため、同一パスの新バイナリが確実に再導入されるよう、成果物 digest を判定へ追加する最小変更を計画する。新旧成果物の入れ替えと対象 service 再起動をテストする。

### 6.3 Inventory、TLS、ネットワーク

生成 inventory は既存の `tugboat_control_plane` と `tugboat_workers` を使う。CSI hostpath は任意の追加デモとし、初期版は `tugboat_csi_hostpath_enabled: false` とする。

| 変数 | デモで指定する値・方針 |
| --- | --- |
| `tugboat_secure` | `true` |
| `tugboat_apiserver_advertise_url` | `https://<cp-management-ip>:8443` |
| `tugboat_apiserver_cert_hosts` / `ips` | localhost、CP 名、127.0.0.1、実際の管理 IP |
| `tugboat_etcd_listen` / `peer_listen` | CP 管理 IP の 2379 / 2380 |
| `tugboat_etcd_advertise_client_url` / `initial_advertise_peer_url` | CP 管理 IP の HTTPS URL |
| `tugboat_etcd_node_name` / `initial_cluster` | CP 名と単一 member の peer URL |
| `tugboat_etcd_endpoints` | ワーカーから到達できる CP の HTTPS endpoint |
| `tugboat_etcd_server_cert_*` / `peer_cert_*` | CP 名と CP 管理 IP を SAN に追加 |
| `tugboat_flannel_mode` | `vxlan` |
| `tugboat_cni_subnet` | 既定候補 `10.244.0.0/16`。ホスト・VM の経路と競合する場合は変更を要求 |
| `tugboat_worker_runtime` / `tugboat_node_name` | `qemu` / ワーカーごとの一意な名前 |

SSH TCP/22、API TCP/8443、ワーカーから CP の etcd TCP/2379、ワーカー相互の VXLAN UDP/8472 を検証する。単一 CP の etcd peer TCP/2380 をワーカーに開放する必要はない。Flannel の実設定でポートを照合する。IP forwarding、必要な kernel module、MTU、ファイアウォールと NAT の影響も検証する。

`static` モードの subnet.env 生成だけで複数ワーカーの通信が完成したとは扱わない。既存 bootstrap による etcd network config と各 flanneld の subnet lease、Node の CNI readiness、ClusterNetworkClass の readyNodes を確認する。

IP 変更時は inventory 更新だけでは不十分。API/etcd の SAN・advertise URL、Flannel endpoint、クライアント設定を再評価する。証明書の CA は維持する方式を検討し、既存 `force-pki` が及ぼす影響を確認する。etcd peer URL は既存 member 更新が必要かを調査する。安全な更新を実装できるまでは、IP 変更を検出して停止し、明示的な作り直し手順を提示する。停止・再開でデータを暗黙に消さない。

### 6.4 クラスタ操作とデモ

1. API の認証付きリソース取得、全期待 Node の登録・状態、Flannel readiness を期限付きで待つ。未実装の `/readyz` に依存しない。
2. 現行の認証実装と PKI/RBAC bootstrap に合わせてデモ操作用の資格情報を発行し、CA 埋め込みの kubeconfig を生成する。agent token の流用や CA 秘密鍵のホスト配布はしない。kubectl の互換性を実機確認し、必要なら操作を VM 内の互換クライアント経由にする。
3. `Imagefile`、`manifests/samples/`、ShipClass/Ship の型定義を参照して最小の起動可能なデモを作る。イメージの取得元・ライセンス・checksum・配布経路を固定し、全ワーカーで利用可能にする。通常の qcow2 を Tugboat の要求形式として無検証で扱わない。
4. 異なるワーカーに割り当てられる 2 つの Ship を用意する。既存のラベル・affinity の対応フィールドを確認して配置し、実際の配置結果も検証する。
5. Running 状態だけでなくゲスト内の起動完了と、片方からもう片方のサービスへの疎通を確認する。ゲスト内操作手段、専用鍵、サービス起動をデモイメージ側で準備する。ワーカー間 ping だけで代替しない。
6. リソース一覧、配置ノード、IP、確認コマンドを表示する。デモ専用 namespace/ラベルに限定した後片付け操作を用意する。

## 7. 再実行・失敗・削除の設計

- state には schema version、UUID、期待 VM 名、作成段階、実 IP、入力 digest、完了段階を保存する。保存は一時ファイルからの rename で行う。
- 同名の無関係な VM を引き取らない。状態が不明・UUID 不一致なら止める。launch 成功直後の中断も復旧できるよう、作成予定を先に保存する。
- `up` の再実行では既存 VM を検査し、不足段階だけ進める。台数削減や資源変更は暗黙に行わず、初期版では明示的な再作成が必要とする。
- 失敗時は VM を残し、cloud-init、Ansible、systemd journal、API/CNI/QEMU の診断情報を収集する。秘密鍵、token、kubeconfig の内容をログに含めない。
- タイムアウト、再試行上限、SIGINT 時の状態保存、同一クラスタの多重実行防止を実装する。
- 削除は state と VM 内 UUID で確認した VM 名を明示して `multipass delete --purge <names...>` を実行する。`--all` や全体に作用する `multipass purge` は使わない。
- 一部削除に失敗した場合は残存 VM を state に残して再試行可能にする。全 VM 削除後に専用秘密情報を削除し、非機密ログは必要に応じて保持する。

## 8. 実装順序と完了条件

| 段階 | 作業 | 完了条件 |
| --- | --- | --- |
| P0: 成立性確認 | Linux 実機で Multipass 3 台、ネスト KVM、SSH、VXLAN 下位ネットワーク、最小 Ship を確認 | 対応環境・必要資源・イメージ・認証方式を確定。未解決事項を記録 |
| P1: VM ライフサイクル | CLI、設定、cloud-init、state、inventory、doctor、stop/start/destroy | 独立クラスタを作成・再実行・削除でき、無関係な VM を変更しない |
| P2: Ansible 接続 | prepare、単一ビルド、配布、既存 site 呼び出し、必要な digest 判定改善 | secure な 1 CP + 2 worker が登録され、再適用で不要な再ビルド・再起動がない |
| P3: デモ | 資格情報、イメージ、Ship 配置・通信、診断収集 | 異なるワーカーの Ship が起動し、ゲスト間の通信を検証できる |
| P4: 品質・案内 | 障害系、実 VM smoke、README、既存導線へのリンク | 新規利用者が README の手順だけで作成から削除まで完了できる |

P0 → P1 → P2 → P3 → P4 の順に進める。P0 でネスト KVM やゲスト通信が成立しない場合は、対応環境や実装前提を修正してから本実装へ進む。

## 9. 検証計画

- 通常 CI: CLI 単体テスト、JSON 設定検証、Ansible syntax-check、追加シェルの構文チェック。Multipass 出力の fixture を使い、IP 複数・欠落、途中失敗、名前衝突、削除対象限定を検証する。
- 既存 installer の回帰: 変更した role に応じて `02-worker-join`、`05-tls-pki`、`07-etcd-client-tls`、`09-flanneld-daemon`、`11-idempotency` を実行する。
- 実 VM smoke: ネスト KVM を利用できる専用 Linux ホストで opt-in 実行する。一般的な CI VM にネスト仮想化があるとは仮定しない。
- 正常系: クリーン環境の `up`、Node/CNI readiness、2 Ship の別ノード配置・ゲスト間通信、同一入力の再適用、停止・再開、削除後の残存確認。
- 更新系: 同一パスに新しい成果物を配布した場合の再導入、変更なしの場合の省略、IP 変更の検出と安全な処理。
- 障害系: KVM 不可、容量不足、cloud-init timeout、SSH 不達、etcd/VXLAN 不達、成果物不正、ビルド失敗、中断後再実行、状態不一致、部分削除失敗。
- 測定: 初回とキャッシュ利用時の所要時間、最大メモリ、ディスク消費、Ship 起動時間を記録して推奨値と timeout を調整する。

受け入れ条件は「3 台の VM を作れた」ではなく、「Ansible による導入、2 台のワーカー上の Ship 起動と通信、再実行・再開、当該環境だけの削除が一連の手順で確認できた」とする。

## 10. ドキュメントと参照資料

実装時に `installer/multipass/README.md` を作り、`README.md`、`README_ja.md`、`installer/ansible/README.md`、`docs/en/installation/ansible.md` から案内する。デモの目的、対応環境、必要資源、取得物、操作例、正常出力、トラブルシュート、削除範囲を掲載する。

以下は 2026-09-05 に確認した公式資料。cloud-init を渡す launch、機械可読の VM 一覧、対象指定の削除を設計の根拠とする。ネスト KVM の可否は資料だけで保証せず P0 で検証する。

- [Multipass: cloud-init を使った VM 作成](https://documentation.ubuntu.com/multipass/latest/how-to-guides/manage-instances/launch-customized-instances-with-multipass-and-cloud-init/): `launch` の資源指定と cloud-init 入力。
- [Multipass: list](https://documentation.ubuntu.com/multipass/latest/reference/command-line-interface/list/): JSON/YAML 出力による VM 情報取得。
- [Multipass: 起動問題の調査](https://documentation.ubuntu.com/multipass/stable/how-to-guides/troubleshoot/troubleshoot-launch-start-issues/): launch timeout 後にも cloud-init が継続する点と ubuntu ユーザーの維持。
- [Canonical Kubernetes: Multipass 手順](https://documentation.ubuntu.com/canonical-kubernetes/main/snap/howto/install/multipass/): 名前を指定した `delete --purge` の使用例。
