# VM イメージ

Tugboat の VM イメージは、単一の boot disk layer を含む OCI artifact です。`Imagefile` でローカル disk file、CPU architecture、disk format を指定します。`FORMAT` を省略した場合は `qcow2` として扱われます。

## Imagefile

```dockerfile
FROM ./ubuntu-server-24.04.qcow2
ARCH x64
FORMAT qcow2
```

```dockerfile
FROM ./ubuntu-server-24.04.raw
ARCH x64
FORMAT raw
```

対応 format:

| Format | OCI layer media type | Cache disk file |
| --- | --- | --- |
| `qcow2` | `application/vnd.tugboat.disk.qcow2.v1+gzip` | `disk.qcow2` |
| `raw` | `application/vnd.tugboat.disk.raw.v1+gzip` | `disk.raw` |

## Build と Push

qcow2 image を build / push する例:

```sh
tugboat-cli build \
  --file Imagefile.qcow2 \
  --tag registry.example.com/team/ubuntu:24.04-qcow2 \
  .
```

raw image を build / push する例:

```sh
tugboat-cli build \
  --file Imagefile.raw \
  --tag registry.example.com/team/ubuntu:24.04-raw \
  .
```

insecure な local registry へ push する場合は `--http` を使います。build command は context directory から disk を読み込み、OCI disk layer として gzip 圧縮し、選択した format を metadata に記録して push します。

## Ship での利用

`Ship` は image tag で OCI artifact を参照します。agent は artifact を pull し、disk layer から format を読み取り、format ごとの disk file として cache し、runtime に `imageFormat` を渡します。

```yaml
apiVersion: v1
kind: Ship
metadata:
  namespace: default
  name: raw-ship
spec:
  image: registry.example.com/team/ubuntu:24.04-raw
  shipClass: lightweight
  runtimeClass: cloud-hypervisor
```

`Ship` manifest に image format field は不要です。Cloud Hypervisor runtime の node に配置する場合は、raw image を指す tag を使ってください。

## Runtime 対応状況

| Runtime | qcow2 boot image | raw boot image | Notes |
| --- | --- | --- | --- |
| QEMU | supported | supported | QEMU launch arguments は `format=qcow2` または `format=raw` を使います。raw-backed VM では internal VM snapshot commands は拒否されます。 |
| Cloud Hypervisor | unsupported | supported | runtime は qcow2 boot image を明示的な error で拒否します。Cloud Hypervisor node では raw image を使ってください。 |

hotplug される block volume は boot image format とは別で、引き続き raw-only です。

古い runtime request との互換性のため、runtime `imageFormat` が省略された場合は `qcow2` として扱われます。新しい agent は OCI artifact から pull した format を常に明示的に runtime へ渡します。

## Agent Cache Layout

sample config では agent image cache の default は `/var/lib/tugboat-agent/images` です。pull 済み image は、pull した layer digest を名前にした directory に保存されます。

```text
<image-cache-dir>/
  <layer-sha256>/
    reference
    metadata.json
    disk.qcow2
```

または:

```text
<image-cache-dir>/
  <layer-sha256>/
    reference
    metadata.json
    disk.raw
```

`metadata.json` には image reference、format、disk filename が入ります。

```json
{
  "image": "registry.example.com/team/ubuntu:24.04-raw",
  "format": "raw",
  "disk": "disk.raw"
}
```

`reference` と `disk.qcow2` だけを持つ従来の qcow2 cache entry も、引き続き `Node.status.images` に報告され利用できます。qcow2 layer media type は変わらないため、既存の qcow2 artifact も引き続き pull できます。

raw image は、対象 node の agent と VM runtime が raw image format support を含む version へ更新された後に rollout してください。古い agent は raw OCI disk layer media type を解釈できません。
