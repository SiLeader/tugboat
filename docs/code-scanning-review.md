# Code Scanning review of PR #129

Reviewed commit: `c83a87a2a89cba6fdcff4b54ae511ed2746f10cf`.
Evidence: GitHub CodeQL 2.26.4 analysis `1729128902`, including its SARIF
`codeFlows`, and the five comments supplied in `review.json`.

The following alerts are false positives. No additional runtime changes are
required for these reported flows.

| Alert | Operation | SARIF source |
| --- | --- | --- |
| [103](https://github.com/SiLeader/tugboat/security/code-scanning/103) | Secret list with parameters | `secret_controller` in `tugboat-agent/src/reconciler/runner.rs:152` |
| [104](https://github.com/SiLeader/tugboat/security/code-scanning/104) | Secret watch | `secret_controller` in `tugboat-agent/src/reconciler/runner.rs:152` |
| [105](https://github.com/SiLeader/tugboat/security/code-scanning/105) | ServiceAccount get | `service_account_api` in `tugboat-controller-manager/src/service_account_token_controller.rs:216` |
| [106](https://github.com/SiLeader/tugboat/security/code-scanning/106) | Secret list | `secret_api` in `tugboat-controller-manager/src/service_account_token_controller.rs:86` and `:123` |
| [107](https://github.com/SiLeader/tugboat/security/code-scanning/107) | Secret delete | `secret_api` in `tugboat-controller-manager/src/service_account_token_controller.rs:357` |

## Why these flows are not disclosures

The sources are API handles or a controller, rather than Secret payloads or
service account token values. `Api<T>` stores a client, an optional namespace,
and `PhantomData<T>`; it does not store a resource value. Each reported flow
propagates from the handle through `self.client` to `TugboatClient.base_url`,
then through `build_url` to the URL argument of a request.

`build_url` clones the configured server URL and sets the resource path.
The callers add resource identifiers and, for list/watch, selectors or a
resource version. These paths do not copy Secret data or token values into
the URL. The reported taint on the API/controller object is therefore not
evidence that the URL contains secret material.

Independently of this false-positive data flow, `resource_client::<T>()`
rejects non-HTTPS base URLs for Secrets and ServiceAccounts before any of
these requests is sent. HTTPS clients are constructed with
`https_only(true)`; reqwest also applies this restriction to redirect targets.
The earlier transport protection remains necessary and is retained.

## Verification

`RUSTC_WRAPPER= cargo test -p tugboat-client` passes all 17 tests, including
anonymous HTTP rejection for Secret and ServiceAccount CRUD, list, watch,
status operations, and token issuance. The tests also check that the HTTPS
client rejects an HTTP request before connecting and that ordinary resources
can still use anonymous HTTP.

No CodeQL suppression, variable renaming, or GitHub alert dismissal was made.
The existing SARIF was inspected; a new CodeQL analysis was not run locally.
