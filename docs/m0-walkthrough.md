# M0: a Rust reading guide

M0 establishes the data contracts and identities later stages can rely on. It does not execute Nexa source. Start with the small pieces, then follow one CLI request through the library.

## 1. An owned type with a checked boundary

Read `src/digest.rs`. `Digest(String)` owns its text, and its field is private. A caller obtains a digest by hashing bytes or parsing a string that must satisfy the format. This makes invalid state harder to introduce than a public string field would.

`as_str(&self) -> &str` borrows the existing text. It does not allocate another string. `verify(&self, bytes: &[u8])` borrows both inputs and returns `Result<()>`: success has no additional value; failure has a structured diagnostic. The `?` operator propagates implementation errors. It is Rust syntax, distinct from Nexa's future Pulse guard syntax.

The raw `abc` test uses a public SHA-256 known vector. Other tests show that adding a newline changes a source blob's identity and that changed bytes cannot verify against the original digest.

## 2. Values represented by enums

Read `src/value.rs`. `Value` can be a string or unit. Pattern matching distinguishes the variants exhaustively. `Unit(())` has an explicit JSON value of null, so it does not accidentally mean an omitted argument or an empty string.

`Identifier` is another owned newtype. It checks its string at the boundary and exposes a borrowed view. Its JSON Schema describes the same allowed shape and reserved-word exclusions. These patterns introduce ownership and validation without requiring a generic arena or a complex lifetime model.

## 3. Separate decoding, normalization, and encoding

Read `src/canonical.rs` and then `src/contracts.rs`.

The JSON visitor rejects duplicate keys before a map could discard the earlier value. It also rejects floats, negative zero, and integers outside signed 64-bit range. The encoder independently applies the same numeric/key rules to already constructed JSON values, sorts object keys, and preserves string scalar sequences.

Generic arrays retain order. A particular wire contract knows which of its arrays are sets or nonsemantic collections. For example, blueprint nodes sort by ID, while input parameter order and receipt history remain meaningful. Duplicate entries are errors, not an instruction to deduplicate silently.

`WireDocument` is a trait for the normalization behavior shared by the closed formats. `decode<T>` is generic over that trait and Serde's decoding traits. It constructs the requested Rust type, then checks its local invariants. It deliberately does not produce a `CheckedProgram`: there is no source resolver/type checker yet.

## 4. Trace one command

Run:

```sh
cargo run --locked -- hash examples/hello/hello.compose.json
```

Then follow this path:

```text
src/main.rs: read the bytes and select document hashing
  -> canonical::parse: preserve/reject input facts before decoding
  -> contracts::normalized_json: select the versioned Rust contract
  -> BlueprintDraft::normalize: check local facts and normalize collections
  -> canonical::encode: produce the exact semantic bytes
  -> Digest::of_bytes: produce a content address
```

Reverse the draft's node array in a temporary copy. Document hashing should give the same result. Change a selected version label and it should change. Generic JSON hashing preserves node-array order, demonstrating why document normalization is a separate stage.

## 5. Kernel identity and generated schemas

`src/kernel.rs` describes two future operations, their types, effects, and scoped capability requirements. It does not implement or dispatch them. The descriptor includes an implementation fingerprint produced by `build.rs` from sorted source files, Cargo inputs, compiler version, target, and relevant build settings. It excludes absolute workspace paths and time. This conservatively changes identity when implementation inputs change; it does not claim reproducible machine-code bytes.

`src/schemas.rs` generates JSON Schemas from the Serde/Schemars wire types. The `schemas/` copies are committed for external tools. A test compares them with fresh generation. Semantic rules such as duplicate node IDs or resolved port compatibility need additional checks beyond schema validation.

## 6. Tests worth reading

Start with these tests in `tests/foundation.rs`:

- `raw_digest_matches_sha256_known_vector_and_detects_tampering`
- `typed_unit_is_explicit_and_different_from_an_empty_string`
- `duplicate_keys_are_rejected_even_when_nested_or_escaped`
- `blueprint_array_order_is_nonsemantic_but_connections_are_semantic`
- `known_shape_errors_are_reported_but_resolution_is_not_fabricated`

Then read `tests/cli.rs`: subprocesses establish that identity does not depend on an existing in-memory object. Invalid input must produce no stdout and a structured error on stderr.

## 7. Explain these decisions before M1

Be able to explain why a digest's inner string is private, when borrowing avoids a copy, how `Result` propagates failures, why a visitor is needed for duplicate keys, why array normalization belongs to the document, and why an ABI descriptor cannot grant permission or prove runtime behavior.

The next step introduces source ownership and enums for tokens/expressions. A compilation session will own source buffers; AST spans can refer to their byte ranges without making every AST type carry a source-text lifetime.
