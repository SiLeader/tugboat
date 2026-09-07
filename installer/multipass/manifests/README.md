# Demo manifests

このディレクトリの ShipClass と Ship template は、demo playbook が API に送るリソースの参照用です。worker 名、namespace、image は cluster ごとに異なるため、実行時には demo playbook が同じ内容を生成します。

2つの Ship は topology.tugboat.cloud/host の required node affinity で異なる worker を指定します。手動で利用する場合は ship-a.yaml.j2 と ship-b.yaml.j2 の値を展開してから、専用 namespace と cluster-network を用意してください。
