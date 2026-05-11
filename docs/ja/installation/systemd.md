# Tugboat systemd インストーラー

このガイドでは、`installer/systemd/` に含まれるインストーラースクリプトを使って、
**systemd** で管理されるベアメタルまたは仮想マシン（Debian / Ubuntu）に Tugboat を
デプロイする方法を説明します。

---

## 目次

1. [概要](#1-概要)
2. [前提条件](#2-前提条件)
3. [コントロールプレーンのインストール](#3-コントロールプレーンのインストール)
   - [3.1 基本（セキュアモード、シングルノード）](#31-基本セキュアモードシングルノード)
   - [3.2 インセキュアモード（TLS なし）](#32-インセキュアモードtls-なし)
   - [3.3 カスタムリッスンアドレスと SAN 名](#33-カスタムリッスンアドレスと-san-名)
   - [3.4 HA etcd クラスター](#34-ha-etcd-クラスター)
4. [ワーカーノードのインストール](#4-ワーカーノードのインストール)
   - [4.1 基本（QEMU、静的 Flannel）](#41-基本qemu静的-flannel)
   - [4.2 Cloud Hypervisor ランタイム](#42-cloud-hypervisor-ランタイム)
   - [4.3 vxlan / host-gw モードの Flannel](#43-vxlan--host-gw-モードの-flannel)
5. [インストール後の作業](#5-インストール後の作業)
   - [5.1 Flannel ClusterNetworkClass の作成](#51-flannel-clusternetworkclass-の作成)
   - [5.2 Flannel ネットワーク設定を etcd に書き込む](#52-flannel-ネットワーク設定を-etcd-に書き込む)
   - [5.3 CSI hostpath ドライバーのインストール](#53-csi-hostpath-ドライバーのインストール)
   - [5.4 RBAC の再ブートストラップ](#54-rbac-の再ブートストラップ)
6. [インストール後のファイル構成](#6-インストール後のファイル構成)
7. [アンインストール](#7-アンインストール)
8. [トラブルシューティング](#8-トラブルシューティング)

---

## 1. 概要

インストーラースクリプトは以下の処理を自動化します。

- Tugboat バイナリのダウンロードまたはビルド
- etcd のインストール（OS パッケージまたはアップストリームのタールボール）
- 自己署名 PKI の生成（CA、サーバー/クライアント証明書）
- テンプレートから TOML 設定ファイルを生成・配置
- システムユーザーの作成とファイルパーミッションの設定
- systemd ユニットの登録と起動
- セキュアモード時の RBAC ブートストラップ（ServiceAccount、ClusterRole、トークン）

スクリプトは Debian または Ubuntu ホスト上で **root** として実行する必要があります。

| スクリプト | 用途 |
|---|---|
| `install-control-plane.sh` | apiserver、scheduler、controller-manager、etcd のインストール |
| `install-worker.sh` | agent、VM ランタイム、CNI プラグインのインストール |
| `bootstrap-flannel.sh` | 稼動中の API サーバーに flannel `ClusterNetworkClass` を作成 |
| `bootstrap-flannel-etcd.sh` | Flannel のネットワーク設定キーを etcd に書き込む |
| `install-csi-hostpath.sh` | Kubernetes CSI hostpath ドライバーのインストール |
| `bootstrap-rbac.sh` | RBAC リソースとサービスアカウントトークンの（再）作成 |
| `uninstall.sh` | サービスの停止とインストール済みファイルの削除 |

---

## 2. 前提条件

| 要件 | 備考 |
|---|---|
| Debian 12 または Ubuntu 22.04 以降 | 依存パッケージのインストールに `apt-get` を使用 |
| root 権限 | 全スクリプトで必須 |
| Cargo + Rust ツールチェーン **または** ビルド済みバイナリ | `--build` でコンパイル、`--use-prebuilt --bin-dir <path>` でスキップ |
| `openssl` | PKI 生成に必要（Debian/Ubuntu では自動インストール） |
| `python3` | RBAC ブートストラップに必要 |
| インターネット接続 | etcd（apt にない場合）、CNI プラグイン、Flannel のダウンロードに必要 |

---

## 3. コントロールプレーンのインストール

### 3.1 基本（セキュアモード、シングルノード）

デフォルトモードでは、自己署名 CA と TLS 証明書を自動生成します。
API サーバーは `0.0.0.0:8443` でリッスンします。

**cargo でバイナリをビルドする場合:**

```bash
sudo installer/systemd/install-control-plane.sh --build
```

**ビルド済みバイナリを使う場合:**

```bash
sudo installer/systemd/install-control-plane.sh \
  --use-prebuilt --bin-dir /path/to/bin
```

スクリプトの処理内容:

1. apt 経由で `ca-certificates`、`curl`、`gettext-base`、`openssl`、`tar`、`wget` をインストール
2. etcd v3.6.10 をインストール（まず `apt-get install etcd-server` を試み、失敗時はアップストリームのタールボールを使用）
3. システムユーザーを作成: `tugboat`、`tugboat-etcd`、`tugboat-apiserver`、`tugboat-scheduler`、`tugboat-controller-manager`
4. `/etc/tugboat/pki/`（Tugboat CA + apiserver 証明書）と `/etc/tugboat/pki/etcd/`（etcd CA + サーバー/ピア/クライアント証明書）に PKI を生成
5. バイナリを `/usr/local/bin/` にインストール
6. `/etc/tugboat/` 以下に設定ファイルを生成
7. systemd ユニットをインストールして有効化: `etcd.service`、`tugboat-apiserver.service`、`tugboat-scheduler.service`、`tugboat-controller-manager.service`
8. API サーバーのヘルスチェックが通るまで待機
9. `tugboat-system` Namespace と、`scheduler`・`controller-manager`・`agent` の ServiceAccount、ClusterRole、ClusterRoleBinding を作成し、サービスアカウントトークンを `/var/run/secrets/tugboat.cloud/serviceaccount/` に書き込む
10. API サーバーを RBAC モードで再起動し、scheduler と controller-manager をトークン認証に再設定

### 3.2 インセキュアモード（TLS なし）

ローカル開発やテスト専用です。API サーバーは `0.0.0.0:8080` で認証なしでリッスンします。

```bash
sudo installer/systemd/install-control-plane.sh --build --insecure
```

### 3.3 カスタムリッスンアドレスと SAN 名

他のマシンから API サーバーにアクセスする場合は、証明書にそのアドレスを追加する必要があります。

```bash
sudo installer/systemd/install-control-plane.sh \
  --build \
  --listen 0.0.0.0:8443 \
  --apiserver-host cp.example.com \
  --apiserver-ip 192.168.1.10
```

主なオプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--listen <addr:port>` | `0.0.0.0:8443`（セキュア）/ `0.0.0.0:8080`（インセキュア） | API サーバーのリッスンアドレス |
| `--secure` / `--insecure` | `--secure` | TLS の有効/無効 |
| `--pki-dir <path>` | `/etc/tugboat/pki` | PKI 出力ディレクトリ |
| `--apiserver-host <name>` | *(ホスト名)* | API サーバー証明書に追加する DNS SAN（繰り返し可） |
| `--apiserver-ip <addr>` | *(なし)* | IP SAN（繰り返し可） |
| `--force-pki` | オフ | 既存の証明書を再生成する |
| `--data-dir <path>` | `/var/lib/tugboat-etcd` | etcd データディレクトリ |

### 3.4 HA etcd クラスター

複数のコントロールプレーンノードで etcd クラスターを構成する場合、
各ノードで固有のアドバタイズ URL とピア URL を指定して
`install-control-plane.sh` を実行します。

**ノード 1（新規クラスター）:**

```bash
sudo installer/systemd/install-control-plane.sh \
  --build \
  --etcd-node-name n1 \
  --etcd-listen 0.0.0.0:2379 \
  --etcd-peer-listen 0.0.0.0:2380 \
  --etcd-advertise-client-url https://192.168.1.10:2379 \
  --etcd-initial-advertise-peer-url https://192.168.1.10:2380 \
  --etcd-initial-cluster "n1=https://192.168.1.10:2380,n2=https://192.168.1.11:2380" \
  --etcd-initial-cluster-state new
```

**ノード 2（既存クラスターへの参加）:**

```bash
sudo installer/systemd/install-control-plane.sh \
  --build \
  --etcd-node-name n2 \
  --etcd-listen 0.0.0.0:2379 \
  --etcd-peer-listen 0.0.0.0:2380 \
  --etcd-advertise-client-url https://192.168.1.11:2379 \
  --etcd-initial-advertise-peer-url https://192.168.1.11:2380 \
  --etcd-initial-cluster "n1=https://192.168.1.10:2380,n2=https://192.168.1.11:2380" \
  --etcd-initial-cluster-state existing \
  --etcd-endpoint https://192.168.1.10:2379 \
  --etcd-endpoint https://192.168.1.11:2379
```

HA etcd オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--etcd-node-name <name>` | `default` | etcd メンバー名 |
| `--etcd-listen <addr:port>` | `127.0.0.1:2379` | etcd クライアントリッスンアドレス |
| `--etcd-peer-listen <addr:port>` | `127.0.0.1:2380` | etcd ピアリッスンアドレス |
| `--etcd-advertise-client-url <url>` | `https://<etcd-listen>` | クライアントへアドバタイズする etcd URL |
| `--etcd-initial-advertise-peer-url <url>` | `https://<etcd-peer-listen>` | クラスターメンバーへアドバタイズするピア URL |
| `--etcd-initial-cluster <members>` | `<name>=<peer-url>` | 全メンバーの `name=url` をカンマ区切りで指定 |
| `--etcd-initial-cluster-state <new\|existing>` | `new` | 新規クラスターは `new`、参加時は `existing` |
| `--etcd-endpoint <url>` | `<advertise-client-url>` | apiserver 設定に書き込む etcd エンドポイント（繰り返し可） |

---

## 4. ワーカーノードのインストール

VM をホストする全ノードで `install-worker.sh` を実行します。
ワーカーノードはネットワーク経由で API サーバーに到達できる必要があります。

### 4.1 基本（QEMU、静的 Flannel）

**セキュアな API サーバー（デフォルト）:**

事前にコントロールプレーンノードから agent のサービスアカウントトークンをコピーします。

```bash
# コントロールプレーンノードで
scp /var/run/secrets/tugboat.cloud/serviceaccount/agent/token worker:/tmp/agent-token

# ワーカーノードで
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url https://192.168.1.10:8443 \
  --ca-cert /tmp/ca.crt \
  --service-account-token /tmp/agent-token
```

> CA 証明書はコントロールプレーンノードの `/etc/tugboat/pki/ca.crt` にあります。

**インセキュアな API サーバー:**

```bash
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url http://192.168.1.10:8080 \
  --insecure
```

スクリプトの処理内容:

1. apt 経由で `ca-certificates`、`curl`、`gettext-base`、`kmod`、`tar`、`wget` をインストール
2. QEMU 依存パッケージをインストール: `qemu-system-x86`、`qemu-utils`、`ovmf`
3. `tugboat` システムユーザーを作成
4. `tugboat-agent` と `tugboat-qemu-runtime` を `/usr/local/bin/` にインストール
5. CNI プラグイン（v1.9.0）と Flannel（v0.28.2）を `/opt/cni/bin/` にインストール
6. ブート時に `/run/flannel/subnet.env` を作成する `/etc/tmpfiles.d/tugboat-flannel.conf` を生成
7. `/etc/tugboat/agent/` と `/etc/tugboat/runtime/` に設定ファイルを生成
8. `tugboat-agent.service` をインストールして有効化

主なワーカーオプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--apiserver-url <url>` | *(必須)* | API サーバー URL（例: `https://192.168.1.10:8443`） |
| `--ca-cert <path>` | *(https の場合必須)* | API サーバーの TLS 証明書を検証する CA 証明書 |
| `--secure` / `--insecure` | `--secure` | サービスアカウントトークン認証または匿名認証 |
| `--service-account-token <path>` | *(--secure 時、トークン未インストールの場合必須)* | agent のサービスアカウントトークンファイル |
| `--node-name <name>` | `hostname -s` | Tugboat に登録するノード名 |
| `--cni-subnet <cidr>` | `10.244.0.0/16` | `/run/flannel/subnet.env` に書き込むサブネット |

### 4.2 Cloud Hypervisor ランタイム

```bash
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url https://192.168.1.10:8443 \
  --ca-cert /tmp/ca.crt \
  --service-account-token /tmp/agent-token \
  --runtime cloud-hypervisor
```

`cloud-hypervisor`（apt または上流のスタティックバイナリ）と
`tugboat-cloud-hypervisor-runtime` をインストールします。
設定ファイルは `/etc/tugboat/runtime/cloud-hypervisor-config.toml` に生成されます。

### 4.3 vxlan / host-gw モードの Flannel

ダイナミックな Flannel バックエンドを使う場合、スクリプトは `flanneld.service` を
インストールし、agent がそれに依存するよう設定します。

```bash
sudo installer/systemd/install-worker.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-url https://192.168.1.10:8443 \
  --ca-cert /tmp/ca.crt \
  --service-account-token /tmp/agent-token \
  --flannel-mode vxlan \
  --flannel-etcd-endpoints https://192.168.1.10:2379 \
  --flannel-etcd-ca /etc/tugboat/pki/etcd/ca.crt \
  --flannel-etcd-cert /etc/tugboat/pki/etcd/client.crt \
  --flannel-etcd-key /etc/tugboat/pki/etcd/client.key
```

Flannel オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--flannel-mode <static\|vxlan\|host-gw>` | `static` | `static` は `subnet.env` を書き込む；`vxlan`/`host-gw` は flanneld を起動 |
| `--flannel-etcd-endpoints <urls>` | *(vxlan/host-gw 時必須)* | flanneld が使う etcd エンドポイント（カンマ区切り） |
| `--flannel-etcd-ca <path>` | *(任意)* | etcd TLS CA 証明書 |
| `--flannel-etcd-cert <path>` | *(任意)* | etcd TLS クライアント証明書 |
| `--flannel-etcd-key <path>` | *(任意)* | etcd TLS クライアント秘密鍵 |

> `--flannel-etcd-ca`、`--flannel-etcd-cert`、`--flannel-etcd-key` は
> 三つ同時に指定するか、全て省略する必要があります。

---

## 5. インストール後の作業

### 5.1 Flannel ClusterNetworkClass の作成

コントロールプレーン起動後、Ship が参照する `ClusterNetworkClass` を作成します。
API サーバーに到達できる任意のマシンで実行できます。

```bash
installer/systemd/bootstrap-flannel.sh \
  --apiserver-url https://192.168.1.10:8443 \
  --name cluster-network \
  --subnet 10.244.0.0/16
```

オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--apiserver-url <url>` | `http://127.0.0.1:8080` | API サーバー URL |
| `--name <name>` | `cluster-network` | `ClusterNetworkClass` リソース名 |
| `--subnet <cidr>` | `10.244.0.0/16` | このネットワーククラスの Pod/VM サブネット |

> スクリプトはべき等です。リソースが既に存在する場合はスキップします。

### 5.2 Flannel ネットワーク設定を etcd に書き込む

`--flannel-mode vxlan` または `host-gw` を使う場合のみ必要です。
flanneld が読み取れるよう、Flannel の設定 JSON を etcd に書き込みます。

```bash
installer/systemd/bootstrap-flannel-etcd.sh \
  --backend vxlan \
  --subnet 10.244.0.0/16 \
  --flannel-etcd-endpoints https://127.0.0.1:2379
```

Tugboat の etcd PKI ファイルが `/etc/tugboat/pki/etcd/` に存在する場合、
TLS 認証情報は `--flannel-etcd-ca`、`--flannel-etcd-cert`、`--flannel-etcd-key` を
指定しなくても自動的に検出されます。

オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--backend <vxlan\|host-gw>` | `vxlan` | Flannel バックエンドの種類 |
| `--subnet <cidr>` | `10.244.0.0/16` | Flannel のネットワーク CIDR |
| `--flannel-etcd-endpoints <urls>` | `https://127.0.0.1:2379` | etcd エンドポイント（カンマ区切り） |
| `--flannel-etcd-ca <path>` | *(自動検出)* | etcd TLS CA 証明書 |
| `--flannel-etcd-cert <path>` | *(自動検出)* | etcd TLS クライアント証明書 |
| `--flannel-etcd-key <path>` | *(自動検出)* | etcd TLS クライアント秘密鍵 |

### 5.3 CSI hostpath ドライバーのインストール

CSI hostpath ドライバーはローカルホストディレクトリを使った `PersistentVolume` の
プロビジョニングを提供します。`provisioner: hostpath.csi.k8s.io` を指定した
`StorageClass` を使う場合にのみ必要です。

```bash
# ソースからビルド（Go ツールチェーンが必要）
sudo installer/systemd/install-csi-hostpath.sh --build

# 既存バイナリを使う場合
sudo installer/systemd/install-csi-hostpath.sh --binary /path/to/hostpathplugin
```

オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--build` | *(デフォルト)* | 上流 CSI ソースから `hostpathplugin` をビルド |
| `--binary <path>` | *(なし)* | 既存のビルド済みバイナリを使う |
| `--version <version>` | `v1.17.0` | ビルドする上流 CSI hostpath ドライバーのバージョン |
| `--node-id <name>` | `hostname -s` | CSI ノード ID |
| `--data-dir <path>` | `/var/lib/tugboat-csi-hostpath` | ボリュームストレージディレクトリ |

### 5.4 RBAC の再ブートストラップ

`install-control-plane.sh --secure` は `bootstrap-rbac.sh` を自動的に呼び出します。
手動実行が必要なのは、ServiceAccount の追加やトークンのローテーション時のみです。

```bash
sudo installer/systemd/bootstrap-rbac.sh \
  --apiserver-url https://127.0.0.1:8443 \
  --ca-cert /etc/tugboat/pki/ca.crt
```

RBAC が既に有効な場合は、既存のトークンを渡して認証します。

```bash
sudo installer/systemd/bootstrap-rbac.sh \
  --apiserver-url https://127.0.0.1:8443 \
  --ca-cert /etc/tugboat/pki/ca.crt \
  --auth-token-path /var/run/secrets/tugboat.cloud/serviceaccount/controller-manager/token
```

オプション:

| オプション | デフォルト | 説明 |
|---|---|---|
| `--apiserver-url <url>` | `https://localhost:8443` | API サーバー URL |
| `--ca-cert <path>` | *(https の場合必須)* | CA 証明書 |
| `--token-output-root <path>` | `/var/run/secrets/tugboat.cloud/serviceaccount` | トークンファイルのルートディレクトリ |
| `--token-owner-group <group>` | `tugboat` | scheduler/controller-manager トークンを読み取れるグループ |
| `--auth-token-path <path>` | *(自動検出)* | RBAC 有効時に使用する既存の Bearer トークン |

### 5.5 高度な RBAC 設定

署名済み JWT トークン、OIDC 統合、監査ログなどの高度な機能を有効にするには、`/etc/tugboat/apiserver/config.toml` を編集します。

#### 署名済み JWT トークン

Tugboat はインストール中に Service Account 署名鍵を自動的に生成します。JWT トークンを有効にするには、以下の設定を確認してください：

```toml
[authentication.service_account]
issuer = "https://apiserver.tugboat.cloud"
signing_key_file = "/etc/tugboat/pki/tugboat-apiserver-sa-signing.key"
signing_algorithm = "RS256"
```

#### OIDC 統合

各 ID プロバイダーに対して `[[authentication.oidc]]` ブロックを追加します：

```toml
[[authentication.oidc]]
issuer_url = "https://dex.example.com"
client_id = "tugboat"
username_prefix = "oidc:"
groups_prefix = "oidc:"
```

#### 監査ログ

監査ログを有効にし、ポリシーを定義します：

```toml
[audit]
enabled = true
log_path = "/var/log/tugboat/audit.log"

[[audit.rules]]
level = "RequestResponse"
verbs = ["create", "update", "patch", "delete"]

[[audit.rules]]
level = "Metadata"
```

変更後は API サーバーを再起動してください：
`sudo systemctl restart tugboat-apiserver`

---

## 6. インストール後のファイル構成

### コントロールプレーン

```
/usr/local/bin/
  tugboat-apiserver
  tugboat-scheduler
  tugboat-controller-manager
  etcd
  etcdctl
  etcdutl

/etc/tugboat/
  apiserver/config.toml           (オーナー: tugboat-apiserver、モード 0600)
  scheduler/config.toml           (オーナー: tugboat-scheduler、モード 0600)
  controller-manager/config.toml  (オーナー: tugboat-controller-manager、モード 0600)
  pki/
    ca.crt / ca.key               (Tugboat CA)
    apiserver.crt / apiserver.key
    etcd/
      ca.crt / ca.key             (etcd CA)
      server.crt / server.key
      peer.crt / peer.key
      client.crt / client.key

/var/lib/tugboat-etcd/            (etcd データディレクトリ)
/var/log/tugboat/                 (コンポーネントごとのログディレクトリ)

/etc/systemd/system/
  etcd.service
  tugboat-apiserver.service
  tugboat-scheduler.service
  tugboat-controller-manager.service

/var/run/secrets/tugboat.cloud/serviceaccount/
  scheduler/token
  controller-manager/token
  agent/token
```

### ワーカーノード

```
/usr/local/bin/
  tugboat-agent
  tugboat-qemu-runtime              (または tugboat-cloud-hypervisor-runtime)
  cloud-hypervisor                  (Cloud Hypervisor ランタイムのみ)

/etc/tugboat/
  agent/config.toml
  runtime/config.toml               (QEMU)
  runtime/cloud-hypervisor-config.toml  (Cloud Hypervisor)
  pki/ca.crt                        (コントロールプレーン CA のコピー)

/opt/cni/bin/                       (CNI プラグインバイナリ)
/etc/tmpfiles.d/tugboat-flannel.conf

/etc/systemd/system/
  tugboat-agent.service
  flanneld.service                  (vxlan / host-gw モードのみ)
```

### CSI hostpath ドライバー

```
/usr/local/bin/hostpathplugin
/var/lib/tugboat-csi-hostpath/
/var/run/csi/
/etc/systemd/system/hostpath-provisioner.service
```

---

## 7. アンインストール

```bash
# コントロールプレーンのみ削除
sudo installer/systemd/uninstall.sh --control-plane

# ワーカーノードのみ削除
sudo installer/systemd/uninstall.sh --worker

# コントロールプレーンとワーカーの両方を削除
sudo installer/systemd/uninstall.sh --control-plane --worker

# etcd データ、agent のイメージキャッシュ、CNI プラグインバイナリも削除
sudo installer/systemd/uninstall.sh --control-plane --worker --purge
```

`--purge` を指定しない場合、データディレクトリ（`/var/lib/tugboat-etcd`、
`/var/lib/tugboat-agent` など）はそのまま残されるため、
再インストール時に再利用できます。

---

## 8. トラブルシューティング

### サービスの状態確認

```bash
systemctl status etcd tugboat-apiserver tugboat-scheduler tugboat-controller-manager
systemctl status tugboat-agent
```

### ログの確認

```bash
journalctl -u tugboat-apiserver --since "5 min ago"
journalctl -u tugboat-agent -f
```

### API サーバーのヘルスチェック

```bash
# セキュアモード
curl --cacert /etc/tugboat/pki/ca.crt https://localhost:8443/healthz

# インセキュアモード
curl http://localhost:8080/healthz
```

### etcd のヘルスチェック

```bash
ETCDCTL_API=3 etcdctl \
  --endpoints https://127.0.0.1:2379 \
  --cacert /etc/tugboat/pki/etcd/ca.crt \
  --cert /etc/tugboat/pki/etcd/client.crt \
  --key /etc/tugboat/pki/etcd/client.key \
  endpoint health
```

### ノード登録の確認

ワーカー起動後、API サーバーに登録されたことを確認します。

```bash
curl --cacert /etc/tugboat/pki/ca.crt https://localhost:8443/api/v1/nodes | python3 -m json.tool
```

### PKI の問題

間違った SAN で証明書が生成された場合は `--force-pki` で再生成します。

```bash
sudo installer/systemd/install-control-plane.sh \
  --use-prebuilt --bin-dir /path/to/bin \
  --apiserver-host myhost.example.com \
  --apiserver-ip 192.168.1.10 \
  --force-pki
```

etcd PKI のみを再生成する場合:

```bash
sudo installer/systemd/setup-etcd-pki.sh \
  --pki-dir /etc/tugboat/pki/etcd \
  --server-host etcd.example.com \
  --server-ip 192.168.1.10 \
  --peer-host etcd.example.com \
  --peer-ip 192.168.1.10 \
  --force
```

### Flannel のサブネットファイルが存在しない

`/run/flannel/subnet.env` がブート時に作成されない場合、手動で作成します。

```bash
sudo systemd-tmpfiles --create /etc/tmpfiles.d/tugboat-flannel.conf
```
