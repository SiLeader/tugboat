# カスタムリソース

Tugboat は Kubernetes と同様に `CustomResourceDefinition` (CRD) によるユーザー定義リソースをサポートします。CRD は group、version、kind、plural、scope、任意の OpenAPI v3 schema 検証、`/status` subresource の有無を定義します。

## 概要

使い方は Kubernetes の CRD に近いです。まず CRD を作成し、API discovery に現れることを確認してから、`/apis/{group}/{version}/...` でカスタムリソースを作成します。最初の実装は最小構成で、単一の served/storage version、schema 検証、Namespaced/Cluster scope、status subresource を扱います。conversion webhook、scale subresource、shortNames、categories、printerColumns は未対応です。

内部では、カスタムリソースは固定 protobuf envelope と raw JSON body として保存されます。これにより、Tugboat の etcd resource store と整合しながら、実行時に定義されるリソース型を扱えます。

## 基本フロー

1. `CustomResourceDefinition` を作成します。
2. discovery で確認します。

```bash
curl -s "$APISERVER_URL/apis/example.com/v1" | python3 -m json.tool
```

3. discovery された API path で custom resource を create/get/patch/watch/delete します。

## CRD マニフェスト例

```yaml
apiVersion: apiextensions/v1
kind: CustomResourceDefinition
metadata:
  name: databases.example.com
spec:
  group: example.com
  names:
    plural: databases
    singular: database
    kind: Database
    listKind: DatabaseList
  scope: Namespaced
  versions:
    - name: v1
      served: true
      storage: true
      schema:
        openApiV3Schema: |
          {
            "type": "object",
            "required": ["spec"],
            "properties": {
              "apiVersion": {"type": "string"},
              "kind": {"type": "string"},
              "metadata": {"type": "object"},
              "spec": {
                "type": "object",
                "required": ["engine", "size"],
                "properties": {
                  "engine": {"type": "string"},
                  "size": {"type": "integer", "minimum": 1}
                }
              },
              "status": {"type": "object"}
            },
            "additionalProperties": false
          }
      subresources:
        status: {}
```

## カスタムリソースのマニフェスト例

```yaml
apiVersion: example.com/v1
kind: Database
metadata:
  namespace: demo
  name: demo-db
spec:
  engine: postgres
  size: 1
```

## OpenAPI v3 schema

Tugboat は CRD version の `schema.openApiV3Schema` で custom resource を検証します。schema はマニフェスト内の JSON 文字列として指定します。`type`、`required`、`properties`、`minimum`、`additionalProperties` など一般的な JSON Schema/OpenAPI v3 keyword を利用できます。

Kubernetes 固有の CRD 拡張、defaulting、conversion webhook、strategic merge、custom resource の動的 OpenAPI document 生成は未対応です。登録済みの custom resource path は `/apis/{group}/{version}` で確認してください。

検証エラーは `422 Invalid` として返り、`details.causes` に field path と検証メッセージが入ります。

## status subresource

CRD version に `subresources.status: {}` を設定すると `/status` route が有効になります。

- 通常の create/update/patch は status を spec から独立して保持します。
- `PATCH /status` は body 内の `status` field のみマージします。`status` 以外の field は無視されます。
- status patch では spec は変更されません。

`subresources.status` が無い場合、`/status` route は `404` を返します。

`subresources.status` が有効な場合、create 時に body に `status` が無ければ apiserver が `{}` を補います。そのため schema で `status.<field>` を `required` にすると、空の `status` が制約を満たさず create が `422 Invalid` で拒否されます。schema 側で `status.*` を optional にするか、controller が `/status` subresource 経由でのみ status を埋めるよう設計してください (spec のみのリクエストが拒否されません)。

## scope

`scope: Namespaced` の resource は次の path を使います。

```text
/apis/{group}/{version}/namespaces/{namespace}/{plural}
```

`scope: Cluster` の resource は次の path を使います。

```text
/apis/{group}/{version}/{plural}
```

Namespaced resource を cluster URL で作成した場合、または Cluster resource を namespaced URL で作成した場合は `400 BadRequest` になります。

## コントローラの実装

Tugboat は custom resource の登録と保存を行いますが、その controller のデプロイには関知しません。controller は別プロセスとして実行し、`tugboat-client` で対象 resource を watch して reconcile してください。

```rust
// 概略です。実際の controller は discovery された API path を list/watch し、
// work queue に積み、/status subresource に状態を patch します。
let client = tugboat_client::TugboatClient::try_new(
    "https://127.0.0.1:6443".to_string(),
    tugboat_client::ClientAuth::None,
    tugboat_client::ClientTlsConfig::default(),
)?;
```

長時間動く controller では、既存の `tugboat-client` reflector と runtime module の利用を優先してください。

## 既知の制約

- version は単一のみ。
- conversion webhook は未対応。
- scale subresource は未対応。
- shortNames、categories、additional printer columns は未対応。
- mutating defaulting は未対応。
- CRD を削除すると API path は使えなくなりますが、既存の custom resource data は将来の garbage collector が実装されるまで etcd に孤児として残ります。
- custom resource が存在する状態で CRD の `scope` を `Namespaced` ↔ `Cluster` に切り替えると、以前の key prefix に保存された data はそのままアクセス不能になります。先に既存の custom resource を削除するか、etcd を手動で掃除してください。
- custom resource は API discovery には出ますが、生成済み `/openapi/v3` schema には動的反映されません。

## トラブルシューティング

`422 Invalid` は CRD または custom resource が検証に失敗したことを表します。次を確認してください。

- CRD の `metadata.name` は `{plural}.{group}` です。
- `spec.group` には `core`、`apps`、`apiextensions` などの組み込み group を使えません。
- `spec.versions` は served/storage の単一 version だけです。
- custom resource body は `apiVersion`、`kind`、scope、OpenAPI schema と一致している必要があります。

CRD 作成直後の `404 NotFound` は、apiserver の watcher が registry を同期する前に起きることがあります。最大 1 秒程度 discovery を retry してください。
