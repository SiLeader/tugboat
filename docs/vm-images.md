# VM Images

Tugboat VM images are OCI artifacts that wrap a single boot disk layer. The `Imagefile` selects the local disk file, CPU architecture, and disk format. `FORMAT` defaults to `qcow2` when it is omitted.

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

Supported formats:

| Format | OCI layer media type | Cached disk file |
| --- | --- | --- |
| `qcow2` | `application/vnd.tugboat.disk.qcow2.v1+gzip` | `disk.qcow2` |
| `raw` | `application/vnd.tugboat.disk.raw.v1+gzip` | `disk.raw` |

## Build and Push

Build and push a qcow2 image:

```sh
tugboat-cli build \
  --file Imagefile.qcow2 \
  --tag registry.example.com/team/ubuntu:24.04-qcow2 \
  .
```

Build and push a raw image:

```sh
tugboat-cli build \
  --file Imagefile.raw \
  --tag registry.example.com/team/ubuntu:24.04-raw \
  .
```

Use `--http` for an insecure local registry. The build command reads the disk from the context directory, gzip-compresses it as the OCI disk layer, and pushes metadata that records the selected format.

## Ship Usage

A `Ship` references the OCI artifact by image tag. The agent pulls the artifact, reads the disk format from the image layer, caches the format-specific disk file, and sends the runtime an `imageFormat` value.

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

No image format field is required in the `Ship` manifest. Use a tag that points to a raw image when scheduling onto a Cloud Hypervisor runtime.

## Runtime Support

| Runtime | qcow2 boot image | raw boot image | Notes |
| --- | --- | --- | --- |
| QEMU | supported | supported | QEMU launch arguments use `format=qcow2` or `format=raw`. Internal VM snapshot commands are rejected for raw-backed VMs. |
| Cloud Hypervisor | unsupported | supported | The runtime rejects qcow2 boot images with a clear error. Use raw images for Cloud Hypervisor nodes. |

Hotplugged block volumes are separate from the boot image format and remain raw-only.

For compatibility with older runtime requests, an omitted runtime `imageFormat` is treated as `qcow2`. New agents always send the explicit format they pulled from the OCI artifact.

## Agent Cache Layout

The agent image cache defaults to `/var/lib/tugboat-agent/images` in the sample config. Each pulled image is stored under a directory named from the pulled layer digest:

```text
<image-cache-dir>/
  <layer-sha256>/
    reference
    metadata.json
    disk.qcow2
```

or:

```text
<image-cache-dir>/
  <layer-sha256>/
    reference
    metadata.json
    disk.raw
```

`metadata.json` contains the image reference, format, and disk filename:

```json
{
  "image": "registry.example.com/team/ubuntu:24.04-raw",
  "format": "raw",
  "disk": "disk.raw"
}
```

Older qcow2 cache entries that only contain `reference` and `disk.qcow2` are still reported in `Node.status.images` and remain usable. Existing qcow2 artifacts continue to pull because the qcow2 layer media type is unchanged.

Roll raw images out only after the node agents and VM runtimes on the target nodes include raw image format support. Older agents do not understand the raw OCI disk layer media type.
