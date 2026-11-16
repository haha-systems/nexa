# From source to a reusable program

The first slice shares one Rust library across source evaluation, memory validation, composition, execution, and inspection. Its public boundaries make the difference between data that has been decoded and a program that has been checked explicit.

## Source: syntax becomes a checked implementation

Start in [source/mod.rs](../src/source/mod.rs), then follow the lexer, parser, and checker in that directory. `check` returns an opaque `CheckedModule`; callers cannot construct its internal checked expressions directly. The checker resolves names, types, callable kinds, registered effects, and the acyclic helper graph before evaluation.

Strings and parameter values are owned Rust data. Source spans use UTF-8 byte offsets. Enums represent expressions and resolved call targets; matching those enums defines evaluation. Named calls preserve source evaluation order before values are arranged into parameter order. The pure evaluator cannot dispatch an action; the host-backed evaluator routes each protected primitive through a `Host` trait.

The [source tests](../tests/source.rs) include changed source behavior, helper calls, scalar escapes, Unicode spans, type/effect failures, and nesting limits. The [source CLI tests](../tests/source_cli.rs) exercise the same boundary through a separate process.

## Effects: the host supplies authority

[runtime.rs](../src/runtime.rs) binds a manifest to a launcher-created execution context. The manifest requests authority; the launcher separately supplies the stdout handle/grant. `RuntimeHost` checks the installed binding and policy immediately before writing, records the decision, and produces structured denials, approval requests, and I/O failures.

Passing a writer as `&mut dyn Write` makes the boundary testable without a real console. The [runtime tests](../tests/runtime.rs) demonstrate missing grants, deny dominance, ask, helper calls, malformed arguments, and partial writer failure. Source code does not pass an actor string that can override the execution context.

## Memories: immutable references carry real code

[memory/store.rs](../src/memory/store.rs) verifies digest-addressed bytes, typed formats, dependency closure, and source contracts on load. [memory/seal.rs](../src/memory/seal.rs) checks source and derives its export without invoking it. Alias/catalogue operations are separated from exact locked lookup.

The store uses atomic installation for immutable objects and guarded atomic sidecar updates for catalogues/evidence. A label may move while the pinned object remains the same. [Memory tests](../tests/memory.rs) exercise fresh-store loading, tampering, source-path confinement, and lock behavior.

## Blueprints: compatibility is checked before execution

[blueprint/checker.rs](../src/blueprint/checker.rs) resolves exact exports, verifies connections and cycles, and re-derives the program's effects and dependency contract. A serialized `derived` summary is an assertion to check, not a shortcut around the checker. Both source and typed construction normalize into the same ProgramSpec.

`CheckedProgram` holds only verified implementations. Its scheduler in [runner.rs](../src/blueprint/runner.rs) selects the lexically smallest ready node, evaluates each declared node at most once, and stops starting work after a non-success outcome. Nested programs share the host context and preserve qualified paths. Receipts derive their principal and program identity from checked state, not caller-supplied display metadata.

[Blueprint tests](../tests/blueprint.rs), [reuse tests](../tests/reuse.rs), and [CLI acceptance tests](../tests/acceptance.rs) cover this combined path. [Inspection tests](../tests/inspection_cli.rs) check bounded pages, stale/query-mismatched cursors, and field projection.

## Reuse and evidence: preserve what was established

[memory/program.rs](../src/memory/program.rs) seals a checked composition as another memory. Its public validator and nested loader share the same export/closure checks. A saved graph does not save authority to execute its effects later.

[evidence.rs](../src/evidence.rs) compares declarative fixtures with actual outcomes. Reports retain exact output bytes and typed expectations/observations; setup failures remain diagnostics rather than disappearing or becoming successful results. Evidence is attached by a separate index so recording another applicable report does not modify executable identity.

Follow [cli_impl.rs](../src/cli_impl.rs) to see the orchestration. The CLI supplies filesystem/resource handles and output paths; application code still crosses the checked runtime boundary for its effects. Use the [runnable demo](slice0-demo.md) to trace an actual assembly through these stages.
