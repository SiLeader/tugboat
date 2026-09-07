# Multipass + Ansible demo

このディレクトリは、Ubuntu 24.04 の Multipass VM 3台で Tugboat の最小クラスタを作る再現可能なデモです。ホストから systemd を操作せず、VM の作成は Multipass、ゲスト設定は Ansible role に分担させます。

標準構成は control plane 1台と worker 2台です。worker 数と Multipass の CPU、メモリ、ディスク、CNI サブネットは JSON 設定または CLI で変更できます。worker は QEMU runtime、ネットワークは VXLAN、CSI hostpath は無効です。

## 前提

ホストには次が必要です。

* Linux x86_64
* Multipass の QEMU backend
* QEMU/KVM と書き込み可能な /dev/kvm
* Git、ansible-playbook、ssh、ssh-keygen、kubectl
* source build を使う場合のゲストからの Rust toolchain 取得経路

最初にホストの検査を実行します。

    python3 installer/multipass/demo.py doctor

doctor は /dev/kvm を明示した最小 QEMU 起動でも確認します。KVM が使えない場合は TCG にフォールバックせず停止します。

## 使い方

設定例をコピーして必要な値を変更します。

    cp installer/multipass/config.example.json /tmp/tugboat-multipass.json

作業ツリーが clean な場合は、指定 commit の source archive を作って control plane 上で一度だけ release build します。

    python3 installer/multipass/demo.py up demo --config /tmp/tugboat-multipass.json

作業ツリーを検証用に使う場合だけ dirty snapshot を明示します。

    python3 installer/multipass/demo.py up demo \
      --config /tmp/tugboat-multipass.json --include-dirty

すでに作成した cluster へ Ansible を再適用する場合は次を使います。

    python3 installer/multipass/demo.py provision demo

状態、開始、停止、診断、demo、kubeconfig は次のコマンドで操作できます。

    python3 installer/multipass/demo.py status demo
    python3 installer/multipass/demo.py stop demo
    python3 installer/multipass/demo.py start demo
    python3 installer/multipass/demo.py collect demo
    python3 installer/multipass/demo.py demo demo
    python3 installer/multipass/demo.py kubeconfig demo --output /tmp/demo.kubeconfig
    python3 installer/multipass/demo.py demo demo --cleanup

破棄対象は state に記録された VM 名だけです。確認を省略する場合も cluster 名を明示した --yes が必要です。

    python3 installer/multipass/demo.py destroy demo --yes

## 接続と状態

cluster ごとの状態は .cache/multipass/<cluster> に保存され、ディレクトリは 0700、秘密情報を含むファイルは 0600 です。SSH 鍵と known_hosts は cluster 専用に生成し、Ansible 実行時は host key checking を有効にします。

各 VM の /etc/tugboat-multipass-owner には cluster UUID が保存されます。既存 VM の所有 UUID が一致しない場合、採用、上書き、削除は行いません。destroy も state にある VM 名を個別に指定するため、他の Multipass VM を巻き込みません。

VM を stop して再開したときに management IP が変わる場合、保存データを暗黙に消去せず needs-recreate として停止します。削除と再作成を明示的に行ってください。

## artifact と demo

通常の build は source archive を control plane に転送し、control plane の /opt/tugboat/bin に生成物をまとめて worker へ配布します。prepare playbook は Ubuntu 24.04 の apt 版 Rust ではなく `rustup` の stable toolchain を使い、toolchain version と各 binary の SHA-256 を manifest に記録します。インストール role の marker には artifact digest を含めるため、生成物が変わった再適用では再インストールされます。--external-prebuilt-dir を使う場合は release binary 5個、サイズ、SHA-256を事前検査し、source build を省略します。

demo は secure=true を前提にし、専用 namespace、ShipClass、ServiceAccount、read-only ClusterRole、ClusterRoleBinding を作成し、TokenRequest API で短命 token を発行します。control plane の bootstrap token や ServiceAccount signing private key はホストへ取り出しません。kubeconfig は CA と専用 user token だけを埋め込み、指定された出力先を 0600 で作成します。

demo-a と demo-b は worker の node label topology.tugboat.cloud/host への required node affinity を持つため、別々の worker に配置されます。`demo` コマンドは配置と runtime 状態に加え、status.ips が提供する Ship guest IP を使った guest-to-guest service probe の成功を必須とします。probe はイメージ固有の SSH 資格情報が必要なため、設定例では無効です。demo-a の image に SSH サーバー（port 22）、ログイン用公開鍵、固定した SSH host key、probe command を用意し、demo-b には検査対象のサービスを用意してください。その後 `demo.guest_probe.enabled` を true にし、`ssh_user`、対応するホスト上の秘密鍵への絶対パス `ssh_identity_file`、demo-a に組み込んだ OpenSSH 公開 host key `ssh_host_public_key` を設定します。秘密鍵と known_hosts は probe の間だけ worker A に 0600 で配置され、終了時に削除されます。外側の Multipass VM 用の鍵は自動転用しません。worker A から demo-a の guest IP に SSH 接続し、`source_command` を demo-a 内で実行します。`peer_address` を `__PEER_IP__` にすると demo-b の guest IPv4 が入り、probe command の `__PEER_IP__` と `__PEER_PORT__` も置換されます。疎通確認を準備できていない場合、`demo` は配置確認だけを成功として扱わずエラーで停止します。

image は通常の container image ではなく、Tugboat の OCI VM image artifact である必要があります。作成方法と qcow2/raw の形式は [VM image guide](../../docs/vm-images.md) を参照し、取得元と digest を環境に合わせて config.example.json に固定してください。image digest を設定すると image reference に @digest を付加します。外部 prebuilt binary は Linux x86_64 ELF、共有 library の解決、実行権限、サイズ、SHA-256 を事前検査します。

## テスト

Multipass daemon を使わない検査は次で実行できます。

    installer/multipass/test/run-smoke.sh

実 VM smoke test は環境依存のため、次を明示的に指定したときだけ実行します。

    TUGBOAT_MULTIPASS_REAL_SMOKE=1 installer/multipass/test/run-smoke.sh

失敗時は Ansible の stdout、stderr、systemd、journal、network 診断が state directory の logs/ に残ります。
