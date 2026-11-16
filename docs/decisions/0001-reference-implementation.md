# Decision 1: Rust reference implementation and explicit identity stages

**Status:** Accepted for M0, 16 November 2026.

## Context

Nexa needs one semantic implementation shared by a CLI, agent tooling, and a future inspector. The project is also intended to build practical Rust experience. The first functional slice should establish contracts and observable behavior without needing a native compiler backend.

## Decision

Use one Rust Cargo package with a library and a thin CLI. Pin the toolchain and Cargo lock. Use enums and owned newtypes for wire facts, `Result` for fallible boundaries, and traits where a shared operation or host boundary benefits from one. Interpret checked source in M1; implement authorized host dispatch in M2.

Identity has three explicit stages:

1. Raw source/artifact identity hashes every byte.
2. Semantic JSON identity canonicalizes keys, strings, integers, and whitespace while preserving array order.
3. Document identity first normalizes schema-defined nonsemantic collections, then uses semantic JSON encoding.

All object decoding first passes through strict JSON parsing so duplicate keys cannot disappear. Schema generation derives from the Rust wire types. Generated schemas are checked in for consumers, with a drift test. Local normalization is not called type checking or execution validation.

The kernel descriptor includes a conservative implementation fingerprint. `build.rs` hashes relative source paths/content, Cargo inputs, compiler identity, target, profile, and relevant flags. It excludes machine-specific workspace paths and timestamps. A change may invalidate identity even when behavior is equivalent; this is preferable to silently presenting changed implementation inputs as identical. This fingerprint does not certify compiler correctness or reproducible binary output.

## Consequences

M0 is usable for format/identity experiments and has no `run`, memory resolver, or source parser. The next stages must establish actual contracts from loaded implementations before anything can become an executable checked program. The implemented wire fields stabilize the Slice 0 profile; later schema changes require explicit compatibility decisions and tests.

The learning guide follows owned strings, borrowed references, enum matching, error propagation, generic traits, serialization, and subprocess tests. The implementation should remain readable as it grows; separate crates and asynchronous machinery should be introduced when a concrete boundary requires them.
