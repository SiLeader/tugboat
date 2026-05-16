# Custom Resources

Tugboat supports user-defined resources through `CustomResourceDefinition` (CRD), similar to Kubernetes. A CRD defines a group, version, kind, plural name, scope, optional OpenAPI v3 schema validation, and whether the custom resource exposes a `/status` subresource.

## Overview

The user-facing flow is intentionally close to Kubernetes: create a CRD, wait for it to appear in API discovery, then create resources at `/apis/{group}/{version}/...`. Tugboat's first CRD implementation is smaller than Kubernetes CRDs: it supports one served storage version, schema validation, namespaced or cluster scope, and status subresources. It does not support conversion webhooks, scale subresources, short names, categories, or printer columns.

Internally, custom resources are stored as a fixed protobuf envelope with the raw JSON body. This keeps etcd storage compatible with Tugboat's resource store while allowing runtime-defined schemas.

## Basic Flow

1. Create a `CustomResourceDefinition`.
2. Confirm the resource appears in discovery:

```bash
curl -s "$APISERVER_URL/apis/example.com/v1" | python3 -m json.tool
```

3. Create, get, patch, watch, and delete custom resources through the discovered API path.

## CRD Manifest Example

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

## Custom Resource Manifest Example

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

## OpenAPI v3 Schema

Tugboat validates custom resources against the CRD version's `schema.openApiV3Schema`. The schema is supplied as JSON text in the manifest. Common JSON Schema/OpenAPI v3 keywords such as `type`, `required`, `properties`, `minimum`, and `additionalProperties` are supported by the apiserver validator.

Unsupported Kubernetes-specific CRD extensions include defaulting, conversion webhooks, strategic merge behavior, and dynamic OpenAPI document publication for custom resources. Use `/apis/{group}/{version}` discovery to inspect registered custom resource paths.

Validation failures return `422 Invalid` with `details.causes` entries that include the field path and validation message.

## Status Subresource

Set `subresources.status: {}` on the CRD version to enable `/status` routes. When enabled:

- normal create/update/patch preserves status independently from spec;
- `PATCH /status` merges into the existing `status` field; non-`status` fields in the
  patch body are ignored;
- status patches do not update spec fields.

Without `subresources.status`, `/status` routes return `404`.

If `subresources.status` is enabled, the apiserver populates `status` with `{}` on
create when the body does not provide one. Schemas that mark `status.<field>` as
`required` will then reject create with `422 Invalid` because the empty default
does not satisfy the constraint. Either leave `status.*` optional in the schema
or have controllers populate `status` exclusively through the `/status`
subresource (so spec-only requests are never rejected).

## Scope

`scope: Namespaced` resources use:

```text
/apis/{group}/{version}/namespaces/{namespace}/{plural}
```

`scope: Cluster` resources use:

```text
/apis/{group}/{version}/{plural}
```

Creating a namespaced custom resource through a cluster URL returns `400 BadRequest`, and creating a cluster-scoped custom resource through a namespaced URL also returns `400 BadRequest`.

## Controller Implementation

Tugboat registers and stores custom resources, but it does not deploy controllers for them. Custom controllers should run as separate processes and use `tugboat-client` to watch resources and reconcile desired state.

```rust
// Sketch only: controllers should list/watch the discovered API path,
// enqueue changes, and patch status through the /status subresource.
let client = tugboat_client::TugboatClient::try_new(
    "https://127.0.0.1:6443".to_string(),
    tugboat_client::ClientAuth::None,
    tugboat_client::ClientTlsConfig::default(),
)?;
```

Prefer the existing `tugboat-client` reflector and runtime modules when building long-running controllers.

## Known Limitations

- single version only;
- no conversion webhook;
- no scale subresource;
- no short names, categories, or additional printer columns;
- no mutating defaulting;
- deleting a CRD makes the API path unavailable, but existing custom resource data remains orphaned in etcd until a future garbage collector removes it;
- changing a CRD's `scope` between `Namespaced` and `Cluster` after custom
  resources exist leaves the original rows under their previous key prefix
  unreachable. Delete the existing resources before flipping scope, or expect
  to clean them up manually from etcd;
- dynamic custom resources are visible through API discovery, not through generated `/openapi/v3` schemas.

## Troubleshooting

`422 Invalid` means the CRD or custom resource did not pass validation. Check:

- `metadata.name` on the CRD must be `{plural}.{group}`;
- `spec.group` cannot be a reserved built-in group such as `core`, `apps`, or `apiextensions`;
- `spec.versions` must contain exactly one served storage version;
- custom resource bodies must match `apiVersion`, `kind`, scope, and OpenAPI schema.

`404 NotFound` immediately after creating a CRD usually means the apiserver watcher has not synchronized the registry yet. Retry discovery for up to one second.
