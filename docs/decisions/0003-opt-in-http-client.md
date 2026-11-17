# Decision 3: an opt-in HTTP GET adapter

**Status:** Accepted for the HTTP client experiment.

## Decision

Test the existing language architecture with a useful networked workflow before expanding its syntax. Add an optional Cargo feature `http-client`, advertising kernel extension `http-client/0.1`. Keep the default local Slice 0 unchanged. Source parsing and blueprint composition use the existing contracts and host dispatch interface.

Use reqwest's blocking client with rustls for HTTP(S), verified TLS, no proxy discovery, no redirect following, and no automatic retries. Return a bounded UTF-8 body from a successful GET. Fix runtime limits at five seconds and 1 MiB for this experiment. Rich request/response types and recoverable source-level errors depend on the general-purpose type foundation.

Derive the actual origin from the URL at each invocation. Require matching application request, launcher grant, and policy before dispatch. Record that concrete origin in decisions/effects. The static `url.origin` requirement describes the argument-dependent resource; it is never a grant or wildcard. Nested memories retain their caller's authority. Authorization is for an origin, not an IP address or individual path.

Include enabled Cargo features in the implementation fingerprint. Require resealing/relocking when the build changes. A feature-enabled kernel is distinguishable from the default kernel even with the same ABI/profile label.

## Format compatibility

The pre-release kernel descriptor gains a defaulted `extensions` list, omitted when empty. The shared receipt/evidence fault enum gains HTTP fault codes. Schema IDs remain 0.3; older closed-schema readers need an upgrade for new faults or extension metadata. Runtime manifest and fixture resource normalization accepts exact HTTP(S) origins only when the adapter is compiled. Generated schemas express wire shape; installed-kernel and authority rules remain semantic checks.

## Evidence

The [experiment guide](../http-client-experiment.md) reproduces the local server workflow. [Transport tests](../../tests/http_transport.rs) cover status, redirects, response bounds, UTF-8, URL validation, and timeouts. [Contract tests](../../tests/http_contracts.rs) check effect metadata and origin normalization. The [subprocess test](../../tests/http_client.rs) covers construction, authorization before dispatch, partial execution, receipts, and nested reuse. The [default-build test](../../tests/http_disabled.rs) checks that the base kernel cannot use the HTTP operation or authority.
