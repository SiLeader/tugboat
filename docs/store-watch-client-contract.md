# Store/Watch/Client Contract

This document fixes the Phase 2 boundary between `tugboat-resource-store`, apiserver watch endpoints, and the `tugboat-client` controller runtime.

## Store Operations

`ResourceStore::get(namespace, name)`:

- Returns `Ok(None)` when the key does not exist. Not found is not a store error.
- Returns the deserialized resource and etcd `mod_revision` as `ContentData` when the key exists.
- Apiserver exposes that revision through `ContentData::apply_revision()` as `metadata.resourceVersion`.

`ResourceStore::list(namespace, limit)`:

- Scans only the namespace prefix when a namespace is provided.
- Scans the full resource type prefix when namespace is omitted.
- Each item resourceVersion is the item's own etcd `mod_revision`; list-level snapshot revision is not exposed yet.
- `limit` maps directly to etcd get limit. Pagination token semantics are undefined.

`ResourceStore::put(value)` / `put_many(requests)`:

- `metadata.name` is required.
- Namespaced resources require `metadata.namespace`; cluster-scoped resources do not include namespace in the key.
- If `metadata.resourceVersion` is present, it must parse as an integer and is used as an optimistic lock against etcd `mod_revision`.
- Stale `resourceVersion` returns `OptimisticLockFailed`.
- Successful writes return the transaction header revision.

`ResourceStore::put_if_not_exists(value)`:

- Creates only when the key does not already exist and returns `Ok(Some(ContentData))`.
- Returns `Ok(None)` when the key already exists. Already exists is not a store error.

`ResourceStore::delete(namespace, name)`:

- Returns `Ok(None)` when the key does not exist. Not found is not a store error.
- Returns the deleted object's previous value and previous `mod_revision` when the key existed.
- Delete watch events use the previous value as their object.

## Watch Semantics

`ResourceStore::watch(resource_version, namespace)`:

- `resource_version == None` watches changes from the current point forward. The store does not synthesize `Added` events for existing objects.
- `resource_version == Some(N)` uses etcd watch start revision `N + 1`, so only changes after the requested revision are emitted.
- `resourceVersion` must be a non-negative integer string. Invalid values return `InvalidField("resourceVersion", ...)`.
- Event order follows etcd watch response order.
- Put events map to `Added` when etcd key version is 1 and `Modified` otherwise.
- Delete events prefer `prev_kv` and map to `Deleted`.
- When a watch stream fails, the internal task reconnects from the revision after the last observed response header revision.

Apiserver watch endpoints:

- Convert store events to NDJSON `ADDED`, `MODIFIED`, and `DELETED`.
- Set object `metadata.resourceVersion` to the event's etcd revision as a string.
- Apply field and label selectors to each event object; non-matching events are skipped.
- Return HTTP 400 `Status` for invalid `resourceVersion`.

## Client Runtime Semantics

`Api<T>::watch(params)`:

- Opens the watch stream before reading the initial list, then emits initial list items as `WatchEvent::Added` before forwarding watch endpoint events.
- Passes `WatchParams::resource_version(...)` as HTTP query `resourceVersion`.

`Controller<T>`:

- `stream` owns watch stream creation and backoff retry.
- `queue` owns per-resource in-flight reconciliation and cancels the previous child token when a newer event for the same resource arrives.
- `runner` owns reconcile result handling, `Action::requeue`, and API read retry/backoff.
- Resource key is `namespace/name`; cluster-scoped or namespace-less resources use `name`. Events missing `metadata.name` run without dedupe.
- Requeue exits when the latest resource is not found. Not found is not retried.
- API read errors and reconciler errors retry with backoff.

## Contract Tests

Representative Phase 2 tests:

- `cargo test -p tugboat-resource-store`
- `cargo test -p tugboat-client`
- `cargo test -p tugboat-integration-tests --test watch`
- `cargo test -p tugboat-integration-tests --test resource_versioning`
