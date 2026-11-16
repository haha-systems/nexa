# Nexa Language Specification

**Version:** 0.3  
**Date:** 16 November 2026  
**Status:** Proposed design specification; local Slice 0 reference implementation; full-language conformance is not claimed

**Supersedes:** Nexa draft 0.1 and draft 0.2  
**Companion:** [Functional Slice Implementation Plan](nexa-functional-slice-plan.md)

**Contents**

- [1. Purpose and status](#1-purpose-and-status)
- [2. Semantic model](#2-semantic-model)
- [3. Source representation and lexical rules](#3-source-representation-and-lexical-rules)
- [4. Types and values](#4-types-and-values)
- [5. Actions, effects, and resources](#5-actions-effects-and-resources)
- [6. Pulse plans and execution](#6-pulse-plans-and-execution)
- [7. Identity, authority, and policy](#7-identity-authority-and-policy)
- [8. Tasks, state, events, and agents](#8-tasks-state-events-and-agents)
- [9. AI inference, claims, and evidence](#9-ai-inference-claims-and-evidence)
- [10. Memories](#10-memories)
- [11. Blueprints and construction by reference](#11-blueprints-and-construction-by-reference)
- [12. The typed program specification](#12-the-typed-program-specification)
- [13. Validation and the meaning of known good](#13-validation-and-the-meaning-of-known-good)
- [14. Compiler-backed inspection](#14-compiler-backed-inspection)
- [15. Human inspection and a future web inspector](#15-human-inspection-and-a-future-web-inspector)
- [16. Modules, packages, and foreign bindings](#16-modules-packages-and-foreign-bindings)
- [17. Standard library and adapters](#17-standard-library-and-adapters)
- [18. Runtime budgets, persistence, and observability](#18-runtime-budgets-persistence-and-observability)
- [19. CLI and tool protocol](#19-cli-and-tool-protocol)
- [20. Source grammar and resolution](#20-source-grammar-and-resolution)
- [21. Worked example: assembling a greeting](#21-worked-example-assembling-a-greeting)
- [22. Conformance and profiles](#22-conformance-and-profiles)
- [23. Compatibility and migration](#23-compatibility-and-migration)
- [24. Slice 0: exact minimum implementation profile](#24-slice-0-exact-minimum-implementation-profile)
- [25. Design decisions and future work](#25-design-decisions-and-future-work)
- [26. Sources and rationale](#26-sources-and-rationale)

## 1. Purpose and status

Nexa is a general-purpose, statically typed, AI-first programming language. It is designed for agents to construct software efficiently and for humans to inspect and understand that software. It combines ordinary computation with typed effects, authorized operations called **Pulses**, and a queryable memory of reusable implementations.

Two requirements govern the design:

1. Nexa should minimize the context needed to understand, safely modify, and verify a program.
2. Nexa should make previously solved problems discoverable, reusable, and verifiable with minimal reconstruction of their original context.

A Nexa program can be handwritten source, a composition of existing memories, or a combination of both. A **blueprint** describes that composition as a typed graph. It is both executable construction data and a human-inspectable description of the program. A validated blueprint can supply much of the content of a new memory.

The words MUST, MUST NOT, SHOULD, SHOULD NOT, and MAY are normative. Examples illustrate the full proposed language. The Slice 0 source/blueprint fixtures are exercised by the reference interpreter; full-language examples outside that profile remain design examples. The full language is specified here at the source, semantic, and tool-interface level. Binary formats, native code generation, a stable foreign ABI, provider APIs, and an exhaustive library catalogue are outside this revision.

The document distinguishes the full design from **Slice 0**, the deliberately smaller implementation profile in Section 24. An implementation of Slice 0 MUST advertise that profile rather than claim full Nexa 0.3 conformance. Unsupported features MUST produce diagnostics rather than acquire approximate semantics.

### 1.1 Design priorities

Correctness, comprehensibility, and predictable authority take precedence over source compression. Token efficiency is measured across a development task: context retrieval, source changes, diagnostics, repair, and review. Character count is not a substitute for that measurement.

The language SHOULD offer one canonical form for each basic construct. Tooling MUST provide a stable formatted representation. A source file and a plain text diff MUST remain useful without an IDE.

Programs that use no inference MUST require no model provider. Ordinary functions and command-line applications MUST require no agent declaration. Events, agents, and distributed services build on the same foundations as other applications.

### 1.2 Evolution from the earlier drafts

| Earlier design | Nexa 0.3 decision |
|---|---|
| Agent/event loop determines program structure | Functions, actions, and task scopes form the application foundation; handlers are entry points. |
| Pulse is mainly a statement of guards and effects | Pulse is a typed plan; execution yields an inspectable receipt. |
| Actor repeated in action arguments | Execution context carries the authenticated principal; arguments carry domain data. |
| Effect checks are largely runtime conventions | Public action signatures declare effects; enforcement also occurs at every protected host boundary. |
| All external values are confidence-bearing claims | Observations and artifacts preserve provenance; assertions use `Claim[T]` with optional confidence. |
| Agent reconstructs a program from files | Compiler-backed inspection exposes contracts, dependencies, effects, and change impact. |
| Reuse mainly through imports | Locked memories and typed blueprints support construction by reference. |
| Author-selected policy guards carry much of authorization | Required policies are attached to protected bindings and cannot be omitted by the caller. |

The identity boundary, fail-closed authorization, inert advisories, explicit nondeterminism, and standalone/embedding parity from draft 0.2 are retained. Contradictions concerning pure output, mutable-state reads, policy fallback, and event-sender authority are resolved below.

## 2. Semantic model

### 2.1 Entities

| Entity | Meaning |
|---|---|
| Value | Typed immutable data, or a reference to an explicitly managed resource. |
| Function | Deterministic computation over explicit immutable inputs. |
| Action | Computation whose signature declares observable effects. |
| Effect | A category of observation or change, such as `io.stdout` or `model.infer`. |
| Capability | Runtime authority to perform an operation on a scoped resource. |
| Pulse | An inert, typed operation plan with intent, inputs, guards, and effects. |
| Invocation | One attempt to execute a Pulse under an authenticated execution context. |
| Receipt | The invocation's outcomes, decisions, effects, and provenance. |
| Blueprint | A typed, inspectable graph of component instances and connections. |
| Memory | An immutable, addressable implementation with a derived contract and locked dependencies. |
| Evidence | An immutable report of a particular validation, test, evaluation, or benchmark. |
| Agent | A bound principal operating tasks with state, tools, inference, and budgets. |

### 2.2 Compilation and execution

```text
source or typed construction request
  -> parse / decode
  -> resolve names and immutable memory references
  -> type-check and derive effects
  -> validate blueprint connections and resource contracts
  -> produce typed Core IR and Pulse plans
  -> bind host resources and the executing principal
  -> authorize and execute protected effects
  -> produce values, receipts, and evidence
```

Core IR represents ordinary computation, control flow, actions, and task scopes. Pulse IR represents semantic plans and their invocation boundaries. Neither requires every arithmetic expression to become a Pulse.

An interpreter, compiled implementation, and embedding MUST preserve the same observable behavior for supported constructs: results, ordered protected effects, authorization decisions, and specified failures. Internal scheduling and optimization may differ only where the program permits it.

## 3. Source representation and lexical rules

Source is UTF-8. Files use the `.nexa` extension. Identifiers in 0.3 are ASCII letters, digits, and underscores, beginning with a letter or underscore. Paths use dots. Comments are `//` to end of line and non-nesting `/* ... */` blocks.

Braces delimit blocks. Newlines delimit statements and declaration fields when outside parentheses and brackets. A semicolon is the explicit statement separator; a comma separates arguments, list elements, and record fields. A newline immediately following an operator or comma, or inside parentheses/brackets, continues the preceding expression. A multiline record also permits newline field separators. A formatter emits newlines rather than semicolons. Newlines between a completed expression and its following postfix operator MUST be rejected, avoiding ambiguous continuation.

This changes 0.2's claim that newlines are entirely insignificant. Explicit boundaries allow predictable formatting without requiring semicolons everywhere.

| Form | Meaning |
|---|---|
| `is` | Type ascription or runtime type test, determined by grammar position. |
| `=` | Binding or mutation; never equality. |
| `==`, `!=` | Equality and inequality. |
| `:` | Record/argument label or Pulse subject separator. |
| `@name` | An identity reference; never authentication by itself. |
| `#name` | A tag. |
| `%0.85` | A fixed-point confidence literal. |
| `21.4$Celsius` | A unit-refined numeric literal. |
| `?` | A Boolean Pulse guard. |
| `=>` | A statement in a Pulse body. |
| `~>` | Inert advisory data. |

Active keywords are `nexa module import export as type alias interface impl fn action effects let var state return if else match for in while break continue true false none unit and or not is main effect requires policy allow deny ask abstain when otherwise identity agent group grant revoke pulse run blueprint input output node connect after set memory on emit task spawn await scope defer`. Additional words in library records are ordinary field names. Unimplemented profile keywords MUST be diagnosed explicitly.

### 3.1 Literals

Strings use double quotes or triple double quotes. Escapes are `\"`, `\\`, `\n`, `\r`, `\t`, and `\u{HEX}` for a Unicode scalar. Source strings are sequences of Unicode scalars; `Bytes` is separate. Plain strings do not interpolate. Prompt interpolation is defined by the inference library.

`Int` is signed 64-bit, with checked overflow. `Decimal` uses exact finite decimal values; addition and multiplication are exact subject to an explicit resource limit. Decimal division requires an explicit scale and rounding mode. `Float64` is IEEE 754 binary64; literals use `f`, such as `3.14f`. There are no implicit lossy numeric conversions. Integer division truncates toward zero; division by zero and checked integer overflow are runtime faults unless handled through checked arithmetic APIs.

`true`, `false`, `unit`, and `none` are literals. Durations use `250ms`, `30s`, or `2h`; time uses `t"2026-11-16T12:00:00Z"`. Durations have checked signed nanosecond representation; a leading minus is an expression operator. `Time` is an instant, and calendar/local-time operations require an explicit timezone library. Reading time is an effect.

Confidence is fixed-point in the closed interval 0..1 with at most four fractional digits. It represents an attributed assessment, not authority or a probability guarantee. Dimensional conversions are explicit; affine units such as Celsius require a conversion with scale and offset. Money is a record with a currency; exchange uses an explicit timestamped rate.

## 4. Types and values

### 4.1 Foundation

Primitive types are `Unit`, `Bool`, `Int`, `Decimal`, `Float64`, `String`, `Bytes`, `Time`, `Duration`, `Identity`, `Tag`, `Confidence`, and `Secret`. Standard constructors include `List[T]`, `Map[K,V]`, `Maybe[T]`, `Result[T,E]`, `Claim[T]`, `Observation[T]`, `Artifact[T]`, `Pulse[T]`, `Receipt[T]`, `Task[T]`, `Stream[T]`, and resource handles.

There is no untyped `Any` escape hatch. Dynamic boundaries use `Dynamic` in the full language; it must be decoded and validated into a concrete type before a policy, action argument, or blueprint connection accepts it. `Dynamic` is not available in Slice 0.

Records and tagged unions are nominal. Structural compatibility of two records does not silently make them the same domain type. Primitive types, instantiated generic types, and port types must match exactly unless an explicit adapter is present.

```nexa
nexa 0.3
module example.invoice

export type Money is {
  amount is Decimal
  currency is Currency
}

export type Currency is GBP | USD | EUR
export type Lookup[T] is found(value is T) | missing
alias CustomerId is String
```

Aliases do not establish a new domain type. Tagged-union constructors are qualified, for example `Lookup.found(value: invoice)`. `Maybe[T]` has `none` and `some(value: T)`; `Result[T,E]` has `ok(value: T)` and `err(error: E)`. `none` may be inferred only when the containing `Maybe[T]` is known. A record value is `Money { amount: 12.50, currency: Currency.GBP }`.

### 4.2 Bindings and mutation

```nexa
let name = "Ada"
let count is Int = 3
var attempts is Int = 0
attempts = attempts + 1
```

`let` is immutable; `var` permits local reassignment. Public signatures declare types; local expressions use inference. Variables must be initialized before use. Local mutation is permitted in a pure function if it cannot escape and affects only that invocation's values. Persistent or shared state is accessed only through effectful actions.

Values have value semantics. Implementations may share immutable storage. User-visible object identity exists only for explicit resource, task, and state handles. There are no unchecked pointers in the language core. Automatic memory management is required; deterministic external-resource cleanup uses task/resource scopes rather than garbage collection timing.

### 4.3 Functions, closures, and interfaces

```nexa
export fn greet(name is String) -> String {
  return "Hello, " + name + "!"
}
```

A `fn` MUST be deterministic over its arguments and captured immutable values. It MUST NOT read shared mutable state, environment, clock, randomness, secrets, or actions. Taking a state snapshot is an action; processing the resulting explicit immutable value may be pure.

Function types are `Fn[A,B]` for unary functions and `Fn[(A,B),C]` for multiple arguments. A closure captures immutable values by value. Generic parameters use brackets after a declaration name. Generic requirements use interfaces:

```nexa
export interface Render[T] {
  fn render(value is T) -> String
}
```

An `impl Render[Invoice] { ... }` supplies the named methods. Implementations are explicit and coherent: at most one implementation of an interface/type pair may be selected in a resolved package graph. Method resolution is static unless a separately declared dynamic interface is used. Dynamic dispatch retains the declared effect bound.

### 4.4 Control flow and errors

Core control flow includes `if/else`, exhaustive `match`, `for`, `while`, `break`, `continue`, and explicit `return`. Conditions are `Bool`; there is no truthiness coercion. Functions return on all paths or have return type `Unit`. An omitted return at the end of a `Unit` body returns `unit`.

```nexa
match result {
  Result.ok(value: invoice) => { return invoice.total }
  Result.err(error: failure) => { return fallback(failure) }
}
```

Matching tagged unions is exhaustive; overlapping unreachable patterns are diagnostics. `x is T` may narrow a declared union or dynamic value within a successful branch. Refinement does not mutate the value. Pattern bindings are immutable.

Expected domain failures use `Result[T,E]`; there is no automatic unwrapping across blueprint ports. Operational faults, such as a broken host binding, exhausted execution budget, or integer overflow, terminate the current invocation with a structured `Fault`. A supervisor may handle that outcome. Arbitrary catchable exceptions are not part of 0.3.

## 5. Actions, effects, and resources

An action may call functions and actions. Its public effect set MUST be declared and MUST contain every transitively reachable effect. Internal action effects may be inferred. Effects on local, nonescaping variables do not enter the set.

```nexa
export action write(message is String) -> Unit effects {io.stdout} {
  return std.console.write_line(message)
}
```

`io.stdout` says what can happen. The host binding for `std.console.write_line` requires capability `console.write` scoped to the supplied stdout resource. Having an effect in a type does not supply that capability. A host primitive is declared with `requires` in its trusted binding metadata; application code cannot invent a privileged implementation by naming a capability.

Effect names are closed over the resolved dependency graph. A source declaration may define an effect name, but a host effect acquires meaning only from an installed host binding. Public action summaries cannot hide effects. Runtime checks remain required for foreign calls, dynamic dispatch, adapters, and host-provided implementations.

A direct action call within an executing action inherits the current execution context. A call with no explicit Pulse boundary is recorded under a compiler-generated invocation with its source span and operation identity. There is no authority-free path around Pulse enforcement.

Resource contracts describe ownership, scope, and lifetime. Handles such as sockets and database transactions cannot be serialized as ordinary data or captured into a portable plan. A plan instead names a required binding that the host resolves at invocation. Moving an owned handle transfers ownership; sharing requires a handle type that explicitly supports sharing. Cleanup runs on normal exit, cancellation, and failure of its scope.

Secrets are opaque handles. They are non-printable, non-serializable, and unavailable to prompt interpolation by default. A secret-aware host operation may consume a handle under a declared capability. Passing the handle does not reveal secret bytes to user source.

## 6. Pulse plans and execution

### 6.1 Construction

`Pulse[T]` is an inert plan that produces a value of type `T` on successful execution. Domain error types are expressed as `Pulse[Result[T,E]]`. A Pulse's static descriptor includes its effect set; generic APIs may constrain that set. Constructing a plan performs no protected effects.

```nexa
let greeting = @app > greet : name {
  ? name != ""
  => return greet(name)
}
```

The actor is an identity reference, intent is a symbol path, and subject is immutable captured data. The body is a deferred block. Its captured values and executable references must satisfy the plan's portability mode. Portable plans contain serializable values and immutable references to compiled operations; arbitrary runtime closures and handles cannot cross a memory or transport boundary.

All `?` guards are pure `Bool` expressions, evaluated left-to-right before the body. A false guard yields `Skipped(guard_false)` and runs no body effect. `require policy p(...)` adds a required policy decision to the invocation; it cannot remove host-required policies. `? policy p(...)` is invalid because a policy returns a `Decision`, not `Bool`.

`=>` statements run in source order. Local bindings created by them are visible to subsequent statements. A final `return` establishes `T`; otherwise the body yields `Unit`. Plans may contain pure computations and action calls, subject to their effects. `~>` accepts a serializable advisory record and never executes it.

### 6.2 Invocation and outcomes

`run plan` is an action expression and yields a `Receipt[T]`. The action's effects include the plan's effects plus receipt handling required by the host. Blueprint invocation constructs a Pulse with an inherited actor selector; invocation binds that selector to the current authenticated principal. An explicit actor selector must be authorized independently.

```text
Receipt[T] {
  invocation_id, plan_digest, program_digest, principal, origin
  status: Succeeded(T) | Skipped(reason) | Denied(reason)
        | AwaitingApproval(request) | Failed(Fault) | Cancelled(reason)
  decisions, effect_records, child_invocations, timestamps
}
```

A receipt is an immutable snapshot. Long-running execution produces a sequence of snapshots/events with the same invocation ID. `AwaitingApproval` is nonterminal only when an approval-capable supervisor retains the continuation. Slice 0 records it and exits without retaining a continuation.

Creating or loading a Pulse does not authorize it. Authorization occurs before each protected effect with the exact arguments and resource binding for that effect. A single up-front approval is insufficient if those values change.

### 6.3 Partial execution

A Pulse is not a distributed transaction. Earlier effects may have occurred when a later effect is denied or fails. Receipts MUST identify completed effects and the stopping point. Effects are not automatically retried. Retry requires an explicit policy and an operation-specific idempotency contract. Compensation is a further authorized operation with its own receipt; it does not erase the original effect.

Plan digests include code/dependency identities, captured inputs, guards, constraints, and configured budgets. Invocation IDs and timing are outside the plan digest. Sensitive captures remain subject to storage policy; a portable plan that cannot lawfully store its captures must remain local or fail construction.

## 7. Identity, authority, and policy

### 7.1 Execution context

```text
ExecutionContext {
  principal: authenticated executor
  origin: optional authenticated triggering principal
  behalf: optional delegating principal
  bindings: host resource handles
  grants: current scoped grants
  prohibitions: explicit denied capabilities
  policy_snapshot: policy IDs and versions
  budget: remaining execution limits
}
```

Event origin does not become execution authority. A handler executes as its registered service/agent principal; an authenticated sender is preserved as origin. A request can carry a separately verified, attenuated grant. Receiving a request alone supplies no permission.

`@app` is a reference resolved to the host-bound application principal. A CLI launcher may bind an application to the process and explicitly supplied resource handles. The same binding rules apply in development and production. Source text, a memory manifest, and an owner field cannot authenticate an identity or mint a grant.

### 7.2 Required decision procedure

Immediately before each protected operation, the runtime MUST:

1. Resolve and authenticate the executing principal and operation binding.
2. Validate an explicit actor selection and any `behalf` delegation.
3. Resolve the operation's capability and resource scope from trusted binding metadata.
4. Apply prohibitions after combining current host grants and valid delegated grants.
5. Check grant expiry, revocation, resource constraints, and available budgets.
6. Check data-release, consent, freshness, and secret-handling obligations at the destination boundary.
7. Evaluate every policy required by the binding and every policy added by the plan.
8. Record the decision and its reasons before dispatching the operation.

The absence of a valid capability, binding, required policy, or allow decision yields `Deny`. A manifest can request authority and restrict policy; it cannot expand the launcher's authority. The runtime must check a current grant/policy version at dispatch. A revocation committed before dispatch prevents dispatch; revocation after dispatch cannot retroactively cancel an external action. A host binding must define its dispatch point.

### 7.3 Policies

Policies are pure decision functions over explicit immutable request and context snapshots. Their decisions are `Allow`, `Deny(reason)`, `Ask(request)`, or `Abstain`. A policy cannot call a clock; the runtime supplies its captured authorization time.

Surface clauses use `allow when condition`, `deny(reason) when condition`, `ask(request) when condition`, and a final `deny(reason) otherwise` or `abstain otherwise`. Within one policy the first matching clause wins; `otherwise` is a fallback, not an unconditional competing denial. Across independently required policies, `Deny > Ask > Allow > Abstain`. An empty/all-abstain set denies.

Human approval binds an authenticated approver to the exact operation/arguments digest, relevant resource versions, policy version, scope, and expiry. Resumption rechecks authority and preconditions. A changed operation requires a new decision. Confidence may inform domain risk policy but never supplies authority by itself.

### 7.4 Data handling

Sensitive values carry release obligations associated with their provenance. Extracting a value from an observation or claim does not erase those obligations. Derived values conservatively inherit obligations from inputs; ordinary concatenation cannot remove them. An explicit declassification/redaction action can establish a narrower obligation under policy and record its evidence. A full implementation MUST enforce this propagation; a profile that lacks it MUST reject sensitive annotations rather than pretend to enforce them.

The runtime can enforce data flow and tool authority. Prompt instructions alone do not guarantee model behavior. Text from memories, tool descriptions, documents, or models is data; it cannot install policies or execute construction commands.

## 8. Tasks, state, events, and agents

Task scopes own child tasks and resources. `spawn` starts a child in the current scope; `await` observes its typed result. Leaving the scope waits for or cancels children according to the scope's explicit policy. Deadlines, cancellation, and bounded channels are standard facilities. Detached tasks require a supervisor binding.

Shared mutable state is accessed through state actions. Immutable snapshots can enter functions. Concurrent updates use explicit transactions, compare-and-set, or isolated actor ownership. No implicit globally mutable variables exist.

An event has ID, typed payload, authenticated origin when available, correlation, causation, and observed time. `on` registers a typed handler under an execution principal. Local `emit` enqueues data; remote transports add authentication and delivery semantics. Ordering, deduplication keys, and capacity are declared by the transport/queue contract. Exactly-once external effects are not assumed.

An agent combines a bound principal, task scope, owned state, incoming events, inference/tool bindings, and budgets. Agents cannot expand their grants by planning or memory retrieval. A service may be long-lived while individual work items remain bounded.

Replay substitutes recorded nondeterministic action results and suppresses external writes. Concurrent replay records the observed schedule or uses a specified deterministic scheduler. Replay with missing required inputs fails with a diagnostic. A replay is distinguished from a fresh validation or performance benchmark.

## 9. AI inference, claims, and evidence

Inference is a standard effect, `model.infer`, with replaceable provider bindings. Typed calls specify the output type, instruction template, explicit context, model configuration, deadline, and budget. A model is a connector; the executing application or agent remains the principal.

```nexa
export action classify(ticket is Ticket) -> Result[Claim[Category], InferError]
  effects {model.infer} {
  return std.ai.infer[Category](classifier, {
    instruction: "Classify this support ticket.",
    context: [ticket],
    max_output_tokens: 500
  })
}
```

Refusal, invalid schema, unavailable provider, cancelled stream, and exhausted inference budget are distinct outcomes. Invalid output is an `InferError` or preserved raw artifact; it is never a well-typed `Claim[T]` with a reduced confidence number. Streaming deltas are partial artifacts, not final validated `T` values.

`Observation[T]` records a value obtained through a host boundary and its origin, time, and obligations. `Artifact[T]` identifies stored content with format, digest, and provenance. `Claim[T]` is an assertion and contains a typed value, asserting source, derivation/evidence references, time, optional expiry, and optional attributed assessment. Confidence assessments include method/source; absence means unknown, not zero.

Schema validity, domain validity, factual support, and permission are different properties. Verification records the specific predicate, verifier identity/version, inputs, and outcome. No general `Verified[T]` constructor is allowed to imply truth beyond that predicate.

Context items may be text, images, audio, structured values, or retrieved artifacts. Release policy checks the actual destination/provider and obligations. Template interpolation uses explicit typed serialization; it never turns context into executable Nexa code.

Model-requested tools produce typed proposals. Each proposal is decoded, checked against allowed operations, budgeted, authorized, and executed under the application principal. A model cannot directly call a host operation. Goal-driven loops explicitly alternate proposal, validation, authorization, execution, and observation; all iterations consume budgets.

AI evaluation is statistical evidence with a dataset/version, scoring method, model/configuration, and sample results. Recorded responses support deterministic regressions but do not establish model-wide correctness.

## 10. Memories

### 10.1 What a memory contains

A memory is an immutable implementation object with a compiler-derived interface. It may export source functions/actions or an already composed blueprint. It is executable only through a supported implementation kind and ABI. Descriptive text is never executable authority.

```text
MemoryRecord {
  schema: "nexa.memory/0.3"
  language: "0.3"
  profile: implementation profile
  implementation: SourceBundle | ProgramSpecReference
  exports: list of derived CallableContract
  dependencies: exact memory / kernel / artifact digests
  source_map: paths and source blob digests
  metadata: problem description, constraints, provenance, reuse terms
}

CallableContract {
  export_name, parameters, result_type
  effects, required_bindings, capability_requirements
  ownership_and_lifetime_constraints
  declared_preconditions, declared_postconditions
}
```

Declared preconditions/postconditions are assertions until supported by a verifier or execution check. The compiler MUST distinguish them from mechanically established facts. Derived contracts must agree with the implementation; importing a memory revalidates its format, identity, dependency closure, ABI, and contract before use.

Memory records reference source blobs rather than require the agent to resend their contents. A memory assembled from a blueprint references the ProgramSpec and component memories. Reuse does not require expanding all dependencies into source text.

### 10.2 Immutable identity and version labels

The immutable address is `sha256:` followed by 64 lowercase hexadecimal digits. It hashes the canonical bytes of the memory record. Referenced source blobs, ProgramSpecs, and dependencies have their own verified digests. An address is immutable; a human-readable name/version label is a lookup alias.

`local.text.trim@1.0.0` can select a candidate during explicit resolution. It MUST be replaced by an exact digest in the lock and resolved ProgramSpec. Compilation in locked mode and program execution MUST NOT silently consult a mutable label or select a newer memory.

Changing implementation, locked dependencies, contract, or included metadata creates a new memory identity. New test evidence is stored as a separate immutable report attached by reference to the existing identity, so running another test does not change executable identity. An annotation index may grow while its referenced reports remain immutable.

### 10.3 Discovery and matching

Memory queries support exact signatures, effects, constraints, provenance, and natural-language problem descriptions. Project, organization, and ecosystem stores are distinct access scopes. Retrieval must honor the caller's access to source and metadata.

The result includes an immutable reference, a compact contract, match explanation, unmet constraints, and applicable evidence. Results distinguish `ExactInterface`, `CompatibleWithAdapter`, and `CandidateNeedsReview`. Semantic similarity cannot establish an exact match.

A matching interface is necessary but not sufficient for correct reuse. Requirements about transactions, ordering, ownership, workload, resource bounds, and environment must also be checked or reported as unresolved. Ranking is advisory; selection becomes part of an explicit construction request and lock.

### 10.4 Project history

A project memory index may reference past implementations, commits, rejected alternatives, tests, and design decisions. Each item records where its assertions came from. The index must not present a deleted implementation or a formerly passing test as the current program without identifying its revision.

Memory is a tool-visible record, not an implicit update to a model's weights or to language semantics. Sharing/publishing a memory is a separate authorized action; local sealing does not publish it. Source and provenance remain available subject to access policy.

## 11. Blueprints and construction by reference

### 11.1 Source form

A blueprint declares external input ports, exactly one result port, component instances, data connections, optional control-order connections, and constant input settings. A multi-value result uses a record type. Components are exports of immutable memories after resolution.

```nexa
nexa 0.3
module example.hello

export blueprint hello {
  input name is String
  output result is Unit

  node clean = memory("local.text.trim@1.0.0", export: "trim")
  node greeting = memory("local.hello.greet@1.0.0", export: "greet")
  node write = memory("local.console.line@1.0.0", export: "write")

  connect input.name -> clean.name
  connect clean.result -> greeting.name
  connect greeting.result -> write.message
  connect write.result -> output.result
}
```

The version labels are unresolved selectors. A corresponding lock must exist before locked composition. No network discovery occurs when a resolved program runs.

Instantiating `hello(name: "  Ada  ")` constructs a `Pulse[Unit]` with inherited actor selection. It does not run the graph. `run hello(name: "  Ada  ")` invokes the plan under the current execution context. Functions may construct a blueprint plan but cannot execute it.

`set node.parameter = constant_expression` supplies a parameter without a connection. Constant expressions are pure and closed over package constants; they do not read runtime state. A node parameter can have one setting or one connection, never both.

### 11.2 Type and graph rules

1. Node IDs are unique identifiers in a blueprint. Ports must exist in the selected export's contract.
2. Every required node parameter is supplied exactly once. There are no implicit missing arguments in 0.3.
3. A source may fan out immutable values to multiple destinations. Every destination has exactly one producer.
4. Source and destination types match exactly. There is no silent conversion, `Result` unwrapping, model invocation, or adapter synthesis.
5. Adapters are ordinary typed components and appear as nodes with their own provenance.
6. The result port is connected exactly once. An input/result cannot be used in the wrong direction.
7. Data connections and `after predecessor -> successor` control edges form one acyclic dependency graph. Cycles are rejected. Iteration belongs inside an explicitly bounded action/workflow component.
8. Every declared node participates in the invocation and executes at most once in this DAG model, even if its result is unused. Pure-node optimization must preserve observable receipts or identify optimized nodes explicitly.
9. A node becomes ready when its inputs and control predecessors complete. The sequential reference scheduler selects the lexically smallest ready node ID. A concurrent profile may execute independent pure nodes or explicitly parallel-safe actions while preserving contracts.
10. The graph's effects, resource requirements, and capability requirements are derived transitively. Unknown requirements cause failure, not an empty effect set.

If a node is denied, skipped, cancelled, or faults, the sequential reference scheduler stops starting nodes and preserves that outcome as the graph outcome. Dependent nodes do not execute. A concurrent profile requests cancellation of remaining work and records any effects that were already dispatched. The graph receipt identifies nodes that completed, the stopping outcome, and nodes not started. A returned domain `Result.err` remains a value; branching on it requires an explicit component. No graph edge silently propagates it as a runtime fault.

### 11.3 Construction commands

Tools accept typed operations such as `AddNode`, `RemoveNode`, `Connect`, `Disconnect`, `SetInput`, `ReplaceMemory`, and `AddOrderEdge`. Each mutation includes the expected blueprint revision. The service applies it transactionally to a draft, returns the new revision and diagnostics, and does not execute it.

Removing a node must explicitly remove or repair its dependent connections. Replacement must recheck ports, effects, resource ownership, and constraints. The compiler must not silently reconnect a same-named port with a different type.

An adapter-generation request is a separate agent/model activity. It produces proposed source that is checked, tested as required, sealed, and added explicitly. A plain `Connect` operation never invents code.

### 11.4 Draft and resolved forms

`BlueprintDraft` may contain symbolic selectors, unbound inputs, and unresolved requirements. It is inspectable but not executable. A `ResolvedProgramSpec` contains exact references, a complete graph, and a derived contract. The resolver produces a lock; the checker certifies a particular resolved revision; execution accepts that revision only.

Full source and the typed construction API MUST lower to the same normalized ProgramSpec. A source map or layout is a sidecar. Rendering and parsing the canonical blueprint source must reproduce the same normalized semantic graph. Comments and visual coordinates may change without changing executable identity.

For offline construction, `nexa.blueprint-draft/0.3` uses the ProgramSpec's `language`, `profile`, `entry`, `inputs`, `output`, `connections`, `order_edges`, `settings`, and `requirements`, but its nodes contain `selector` in place of the resolved `memory` digest. It has no caller-supplied `derived` contract or kernel digest. The resolver supplies those from the lock and checked implementations. Slice 0 accepts this complete draft through `resolve --request` and `compose --request`; incremental editing commands remain a later API layer. Source and decoded requests must normalize to the same graph under the same compiler-selected profile.

## 12. The typed program specification

The blueprint is both a description and typed construction data. The canonical schema is `nexa.program/0.3`:

```text
ResolvedProgramSpec {
  schema, language, profile, entry
  inputs: List[Port]
  output: Port                       // name == "result"
  nodes: List[Node]
  connections: List[Connection]
  order_edges: List[OrderEdge]
  settings: List[InputSetting]
  kernel: immutable kernel descriptor digest
  requirements: List[Requirement]
  derived: ProgramContract
}

Port { name, type }
Node { id, memory, export }
Connection { from: Endpoint, to: Endpoint }
Endpoint { kind: "input" | "node" | "output", node?: ID, port }
OrderEdge { before: NodeID, after: NodeID }
InputSetting { node, port, value: TypedConstant }
```

For a connection source, endpoints are `input.<name>` or `<node>.result`. For a destination, endpoints are `<node>.<parameter>` or `output.result`. No extra endpoint fields are accepted. A setting cannot target the output. Array order does not express execution order; explicit dependency edges do.

`ProgramContract` contains input/result types, exact dependency closure, effects, capability/resource requirements, and unresolved semantic requirements. Type references use a canonical type expression: built-ins such as `String`; constructors such as `List[String]`; or nominal types qualified by the defining immutable memory and fully qualified type name. Unresolved names/type variables cannot occur in a resolved object. The checker recomputes every derived field and rejects mismatches.

Requirements distinguish structural checks, declared semantic constraints, and empirical targets. A requirement records an ID, kind, subject, predicate/metric, target, unit where applicable, and applicability conditions. Structural checks can be satisfied by the checker. A semantic assertion without an installed verifier remains unresolved. Performance targets require benchmark evidence. `compose` must not turn unresolved requirements into satisfied ones.

Example: a throughput target of 1,000,000 requests/s is meaningful only with workload, concurrency, latency/error bounds, environment, and measurement procedure. Previous evidence ranks candidates; a new assembly needs evidence for its own digest and environment.

### 12.1 Canonical encoding and integrity

The content-addressed formats use UTF-8 canonical JSON, with these rules:

- Object keys are ASCII schema keys or ASCII source identifiers and are sorted by byte order. Unknown fields in closed semantic records are rejected. User metadata lives in a defined extension map.
- No insignificant whitespace or final newline is included in hashed bytes. Strings preserve scalar sequences without Unicode normalization. Non-ASCII scalars are emitted as UTF-8. Quotes and backslashes are escaped; control characters use lowercase `\u00xx`, except the canonical short escapes `\b`, `\t`, `\n`, `\f`, `\r`.
- JSON integers are signed 64-bit decimal with no leading zero or negative zero. Exact decimals and floats in typed constants use canonical tagged string representations; JSON nonfinite numbers are invalid.
- Program nodes sort by ID; connections sort by `(from.kind, from.node-or-empty, from.port, to.kind, to.node-or-empty, to.port)`; order edges sort by `(before, after)`; settings sort by `(node, port)`; requirements sort by ID. Inputs retain declared parameter order. Effect/capability sets sort by their canonical identity.
- Duplicate object keys, IDs, set entries, connections, or conflicting settings are errors, never silently discarded.
- Source blobs hash their exact distributed bytes. The formatter emits UTF-8 with LF; sealing never changes source bytes silently.

The digest hashes the semantic object's canonical bytes and is held in its external envelope/address, not a self-referential field inside the hashed object. Every referenced blob is verified recursively. Cache entries are keyed by semantic digest, compiler/profile version, target, kernel, and build options; evidence has its own applicability key.

### 12.2 Round trips and memory creation

A checked ProgramSpec can be sealed as a memory. Its external inputs become callable parameters; its output becomes the return type; its derived effects and binding requirements become the export contract. Its nodes supply dependency references. Source/provenance sidecars are attached through explicit artifact references.

This process fills the mechanical parts of a memory. A human or agent may add the problem description, domain assumptions, reuse guidance, and evidence links. Those claims do not become compiler facts simply because they are stored next to a typed graph.

To prevent recursive identity definitions, a memory's internal names resolve relative to that memory's compilation unit; its own digest is not embedded into its pre-hash manifest. Cross-memory dependencies must form an acyclic resolution graph. Ordinary function recursion within a memory is permitted by the full language and subject to execution budgets.

## 13. Validation and the meaning of known good

There is no universal `known_good: true` property. A validation report states which claim is supported:

| Evidence | Establishes |
|---|---|
| Checked | This revision parses, resolves, type-checks, and satisfies structural/effect rules under the named profile. |
| Tested | The named fixtures passed for this implementation/dependency/environment revision. |
| Verified predicate | A particular property was established by the named verifier and assumptions. |
| Benchmarked | The recorded metric was observed for a specified workload/environment. |
| Reviewed | A named reviewer recorded an assessment of this revision. |

Reports identify subject digest, tool/verifier version, profile, test inputs or their digests, environment, outcome, time when available, and limitations. Evidence may be signed; an unsigned local report is not a third-party attestation. Reports do not supply runtime permissions.

Composition reuses applicable evidence for unchanged components. It does not inherit a blanket proof for the assembly. The assembly requires its own structural checks and integration tests. Replacing a dependency invalidates evidence keyed to the previous assembly. Negative evidence and regressions remain queryable.

## 14. Compiler-backed inspection

Inspection is a required semantic API, usable through the CLI, editor language server, or an agent transport. It exposes the same compiler model, rather than a separately maintained explanation.

Required queries are `InspectSymbol`, `FindReferences`, `InspectProgram`, `ExplainEffects`, `ExplainAuthority`, `InspectMemory`, `FindMemories`, `ExplainConnection`, `PreviewChange`, and `CheckEdit`. A partial profile advertises the subset it implements.

Requests include protocol version, workspace revision or object digest, query, field projection, page size, and optional cursor. Responses include status, exact revision, records, diagnostics, omitted-field indicators, next cursor, and invalidation/dependency information. Stable symbol identity is `(memory or workspace revision, qualified declaration path)`; source offsets alone are not identity.

The default result is a bounded summary. Bodies, full dependency closures, historical records, and evidence payloads require explicit expansion. Defaults MUST NOT send entire source trees to the agent. Pagination is deterministic for a fixed revision; stale revisions/cursors are diagnosed.

Diagnostics include code, severity, message, source span or object path, expected/actual type or requirement, related locations, and a suggested repair when available. Suggested repairs include a base revision and are never applied as executable instructions from diagnostic text.

`ExplainAuthority` describes required operations, scoped capabilities, and policy bindings. A static explanation cannot promise runtime allowance; it identifies facts requiring host evaluation. `CheckEdit` stages edits and returns diagnostics without executing the program or performing protected effects.

## 15. Human inspection and a future web inspector

The inspector displays the typed blueprint graph. A node exposes its contract, implementation reference, source, configuration, effects, resource assumptions, and evidence. Connections expose source/destination types and any adapter. A program panel shows unresolved requirements and the exact revision being viewed.

Layout, collapsed groups, colors, and coordinates are presentation sidecars. They do not affect program identity or hide semantic nodes. A composed memory can be expanded recursively, subject to access policy. The view must distinguish declared requirements, statically checked properties, and measured evidence.

Edits use the transactional construction API from Section 11.3. A visual edit and a source edit are checked by the same compiler and produce the same ProgramSpec. An inspector must not silently execute a plan while editing it. Running a program is a separate operation under an execution context.

The first implementation exposes JSON inspection and canonical source; a browser UI is deferred. The schema must nevertheless retain enough identity and type information for that UI without reconstructing semantics from screenshots or rendered text.

## 16. Modules, packages, and foreign bindings

Files begin with `nexa 0.3` and a module path. Declarations are private unless `export` is present. Imports bind exported declarations from an explicit dependency alias. A package manifest maps aliases to selectors; its lock maps aliases to immutable identities. Ambiguous symbols require qualification. No import performs runtime effects.

Functions and actions have distinct callable kinds. A pure function cannot accept an unconstrained action where a function is required. Effectful higher-order APIs constrain callable effects; generic specializations retain the effect union of their supplied operations.

Foreign bindings register a typed signature, effects, scoped capability mapping, ABI/version, failure behavior, resource ownership, serialization rules, and idempotency/compensation description when relevant. Memory code can reference such a binding but cannot install an arbitrary host function through an import. Unsafe/native bindings require explicit host installation and remain visible to inspection.

Host embedding supplies the same checked program objects, binding API, authorization procedure, and receipts. A host cannot label an unchecked callback as pure. Portability means preserved semantics for a declared binding contract, not that all hosts provide every integration.

## 17. Standard library and adapters

Required full-language library families are collections, text/bytes, checked numeric operations, exact decimals, time/calendar, results, serialization, resources, tasks/channels, state, effects, identity/policy, Pulse/receipts, memory/blueprints, inspection, and inference contracts. Concrete network/database/model/MCP providers are optional adapters.

`std.text.trim_ascii(text)` is pure and removes only leading/trailing U+0009 through U+000D and U+0020. It does not normalize or strip other Unicode characters. `std.console.write_line(text)` is an action with effect `io.stdout`, capability `console.write` scoped to stdout, and result `Unit`. It writes UTF-8 plus one LF; write failure is an operational fault and may follow partial output. `print` is not a pure function.

MCP tools, HTTP operations, database calls, and model connectors share action, effect, provenance, and authorization contracts. Connector-specific protocols remain versioned dependencies. Dynamic tool discovery produces untrusted metadata and proposed interfaces; using a changed interface requires validation and, for portable programs, a new lock. This specification does not select a current MCP protocol version.

## 18. Runtime budgets, persistence, and observability

An invocation receives limits for instructions, wall time, memory, task count, model tokens/cost, retries, and effect counts as applicable. Effective child budgets cannot exceed the parent's remaining limits. An unbounded service lifetime does not imply unbounded per-item work.

The runtime records invocation start/finish, node start/finish, policy decisions, effect dispatch/result, task cancellation, and evidence references. Receipts propagate correlation/causation and exact code/program identities. Argument/result recording follows data policy; a public receipt may contain redactions and digests rather than raw values. Authorization reason codes remain available without exposing secrets.

Persistent supervisors store continuations, state versions, outbox entries, and effect idempotency keys at documented boundaries. A host operation may report outcome unknown after a crash; automatic reissue is forbidden unless the operation's retry contract permits it. A trace is not sufficient to guarantee exactly-once behavior in an external system.

Operational timestamps are observations, not source-level calls to a pure clock. Their recording is provided by the host and identified as nondeterminism. Program identity excludes timestamps, invocation IDs, and machine paths.

## 19. CLI and tool protocol

The reference CLI uses the following conceptual surface. Unsupported profile commands MUST report a structured unsupported-feature error.

```text
nexa fmt <source>
nexa compile <source> --entry <entry> --out <program.json>
nexa check <source-or-program> --profile <name> --json
nexa memory seal <source-directory> --store <path>
nexa memory seal --program <program.json> --name <name> --version <version> --store <path>
nexa memory inspect <digest> --store <path> --json
nexa memory find <query> --store <path> --json
nexa resolve <source> --store <path> --write-lock <lockfile>
nexa resolve --request <draft.json> --store <path> --write-lock <lockfile>
nexa compose <source> --locked --lock <lockfile> --store <path> --out <program.json>
nexa compose --request <draft.json> --locked --lock <lockfile> --store <path> --out <program.json>
nexa inspect <program.json> --store <path> --json
nexa run <program.json> --input-file <json> --manifest <json> --allow <resource> --receipt <json> --store <path>
nexa test <program-or-memory> --fixtures <json> --store <path> --report <json>
nexa benchmark <program> --workload <json> --report <json>
nexa replay <receipt-or-trace>
```

`--allow stdout` supplies only the launcher's stdout handle and associated capability; it is not a general policy bypass. Manifest requests must remain within that supplied authority. `resolve`, `compose`, `check`, and `inspect` MUST NOT invoke application actions. Package/network retrieval requires a separately declared tool permission and is outside Slice 0.

`run` reserves stdout for program output. Diagnostics go to stderr; the receipt goes to the specified file. In the full language a program result is also available through its receipt. Slice 0 exits 0 for `Succeeded`, 1 for a runtime `Failed`, 2 for construction/check errors, 3 for `Denied`, 4 for `AwaitingApproval`, and 5 for `Skipped` or `Cancelled`. The CLI MUST not mix a JSON receipt into application stdout.

`--json` queries produce UTF-8 JSON with protocol version and profile. Commands that access memories accept the common `--store` option; test commands accept an application `--manifest`. Invocation IDs may be generated by the host; deterministic compilation does not require deterministic IDs. CLI implementation and agent/editor transports must call the same semantic library functions.

## 20. Source grammar and resolution

The following grammar fixes the core forms. `NL` is a significant newline under Section 3; `sep` is one or more newlines or a semicolon. Commas may replace newlines between record fields. Comments are removed without removing their newline boundaries. Operator precedence is highest at postfix access/call, then unary, multiplicative, additive, comparison/type test, `and`, and `or`. Comparisons do not chain. Assignment is a statement, not an expression.

```ebnf
file          = "nexa", VERSION, sep, "module", path, sep, { declaration, sep } ;
declaration   = [ "export" ], (type_decl | alias_decl | fn_decl | action_decl
              | interface_decl | impl_decl | blueprint_decl | policy_decl
              | identity_decl | agent_decl | handler_decl | main_decl)
              | import_decl ;
import_decl   = "import", path, [ "as", ident ] ;
type_decl     = "type", ident, [ type_params ], "is", (record_type | variants) ;
alias_decl    = "alias", ident, [ type_params ], "is", type_expr ;
variants      = variant, { "|", variant } ;
variant       = ident, [ "(", parameters, ")" ] ;
record_type   = "{", fields, "}" ;
fields        = field, { field_sep, field }, [ field_sep ] ;
field         = ident, "is", type_expr ;
type_params   = "[", ident, { ",", ident }, "]" ;
type_expr     = type_atom, [ "$", ident ], [ "effects", effect_set ] ;
type_atom     = path, [ "[", type_expr, { ",", type_expr }, "]" ]
              | "(", type_expr, ",", type_expr, { ",", type_expr }, ")" ;
parameters    = parameter, { ",", parameter } ;
parameter     = ident, "is", type_expr ;
fn_decl       = "fn", ident, [ type_params ], "(", [ parameters ], ")",
                "->", type_expr, block ;
action_decl   = "action", ident, [ type_params ], "(", [ parameters ], ")",
                "->", type_expr, "effects", effect_set, block ;
effect_set    = "{", [ path, { ",", path } ], "}" ;
interface_decl = "interface", ident, type_params, "{", { fn_signature, sep }, "}" ;
fn_signature  = "fn", ident, "(", [ parameters ], ")", "->", type_expr ;
impl_decl     = "impl", type_expr, "{", { fn_decl, sep }, "}" ;
block         = "{", { statement, sep }, "}" ;
statement     = ("let" | "var"), ident, [ "is", type_expr ], "=", expr
              | lvalue, "=", expr
              | "return", expr
              | "if", expr, block, [ "else", (block | if_statement) ]
              | "match", expr, "{", { pattern, "=>", block, sep }, "}"
              | "for", ident, "in", expr, block
              | "while", expr, block | "break" | "continue"
              | "scope", block | "defer", block
              | "emit", expr | expr ;
main_decl     = "main", block ;
handler_decl  = "on", path, "as", ident, block ;
identity_decl = "identity", identity, "is", path, record_value ;
agent_decl    = "agent", identity, record_value ;
policy_decl   = "policy", path, "(", [ parameters ], ")", "{",
                { decision, "when", expr, sep }, decision, "otherwise", sep, "}" ;
decision      = "allow" | "abstain" | "deny", "(", expr, ")"
              | "ask", "(", expr, ")" ;
blueprint_decl = "blueprint", ident, "{", { blueprint_item, sep }, "}" ;
blueprint_item = "input", ident, "is", type_expr
              | "output", "result", "is", type_expr
              | "node", ident, "=", "memory", "(", string,
                  ",", "export", ":", string, ")"
              | "connect", endpoint, "->", endpoint
              | "after", ident, "->", ident
              | "set", ident, ".", ident, "=", constant_expr ;
endpoint      = ident, ".", ident ;
pulse_expr    = identity_or_path, ">", path, ":", expr,
                "{", { pulse_item, sep }, "}" ;
pulse_item    = "?", expr | "=>", statement | "~>", record_value
              | "require", "policy", call ;
identity_or_path = identity | path ;
call          = path, [ type_args ], arguments ;
expr          = pulse_expr | logical_or ;
logical_or    = logical_and, { "or", logical_and } ;
logical_and   = comparison, { "and", comparison } ;
comparison    = additive, [ ("==" | "!=" | "<" | "<=" | ">" | ">="), additive
                         | "is", type_expr ] ;
additive      = product, { ("+" | "-"), product } ;
product       = unary, { ("*" | "/"), unary } ;
unary         = ("not" | "-" | "run" | "spawn" | "await"), unary | postfix ;
postfix       = atom, { ".", ident | type_args | arguments } ;
atom          = literal | ident | identity | tag | list | record_value
              | path, record_value | "(", expr, ")" | lambda ;
lambda        = "fn", "(", [ parameters ], ")", "->", type_expr, block ;
arguments     = "(", [ argument, { ",", argument } ], ")" ;
argument      = [ ident, ":" ], expr ;
type_args     = "[", type_expr, { ",", type_expr }, "]" ;
list          = "[", [ expr, { ",", expr } ], "]" ;
record_value  = "{", [ ident, ":", expr, { field_sep, ident, ":", expr } ], "}" ;
pattern       = "_" | literal | ident | path, "(", [ pattern_fields ], ")" | path ;
pattern_fields = ident, ":", pattern, { ",", ident, ":", pattern } ;
path          = ident, { ".", ident } ;
identity      = "@", ident ;
tag           = "#", ident ;
```

`literal`, `string`, `ident`, `VERSION`, and field/statement separators follow Section 3. A final statement/field before `}` may omit its separator. Empty record types/values are permitted. `lvalue` is a local variable or its assignable value field; it never grants direct mutation of a shared handle. `if_statement` is the `if` statement production. `constant_expr` excludes variable captures and actions. Keywords `input` and `output` are accepted only as the corresponding special endpoint owners; user node IDs cannot use them.

Generic type arguments on a call are recognized only after resolving its callee as a generic callable; there is no `<...>` generic ambiguity with comparisons. Named and positional arguments may not be mixed in ordinary calls. The blueprint `memory(selector, export: name)` declaration is a grammar-defined constructor, not an ordinary call. Pulse construction is parsed only at an expression root beginning with an identity or resolved identity path; `>` elsewhere is comparison. A nested comparison or record construction in a Pulse subject must be parenthesized; the first unparenthesized opening brace terminates that subject and starts the Pulse body. The parser produces an explicit unresolved Pulse/comparison form when name resolution is required and must diagnose ambiguity rather than guess.

Keywords reserved for effect registration, delegation, and persistent state (`effect`, `requires`, `grant`, `revoke`, `state`) have no ordinary application declaration syntax in 0.3; those operations use trusted descriptors or standard actions. The `pulse` keyword is reserved for tooling renderings. Identity/agent declarations describe requested host bindings and ownership; their record fields cannot supply authenticated handles.

`main { ... }` is shorthand for the module's parameterless application action returning `Unit`, with effects inferred from its body. It is the one public-entry exception to the explicit action-effect rule; the compiler exposes the inferred contract. `nexa compile <source> --entry main --out <program.json>` seals the entry implementation and constructs a one-node ProgramSpec using the same pipeline as explicit blueprint composition. A blueprint-only file needs no `main`.

## 21. Worked example: assembling a greeting

Three memories contain these implementations. Each source file has `nexa 0.3`, its own module declaration, and the indicated exported callable.

```nexa
// module local.text.trim
export fn trim(name is String) -> String {
  return std.text.trim_ascii(name)
}
```

```nexa
// module local.hello.greet
export fn greet(name is String) -> String {
  return "Hello, " + name + "!"
}
```

```nexa
// module local.console.line
export action write(message is String) -> Unit effects {io.stdout} {
  return std.console.write_line(message)
}
```

The blueprint in Section 11 connects them. For input `{"name":"  Ada  "}`, the program writes exactly the UTF-8 bytes `Hello, Ada!\n` and returns `unit` through its receipt. Pure nodes require no capabilities; the write requires the host-supplied stdout capability and a matching allow policy.

If stdout permission is absent, the graph may complete its pure nodes but emits no greeting; the write node is denied and the invocation records `Denied`. If the `write.result` port of type `Unit` is connected to `greeting.name` of type `String`, composition fails before any execution.

Saving this ProgramSpec as `local.hello@1.0.0` produces a new memory exporting `hello(name is String) -> Unit` with effect `io.stdout`. A second blueprint may select that single export. It must still obtain stdout permission at execution. Both assemblies should produce the same greeting and leaf effect, although their node hierarchy and program digests differ.

This is the reference first slice. It deliberately exercises a real source parser, source evaluation, typed composition, an authorized effect, inspection, receipts, and reuse of a composed memory without needing a network service or a model provider.

## 22. Conformance and profiles

Full Nexa 0.3 conformance requires the general-purpose foundation, effect system, Pulse semantics, authority procedure, blueprint/memory formats, semantic inspection contract, structured task/state semantics, and inference contracts described here. A concrete provider, remote registry, web UI, or native compiler backend is not required. Unsupported adapters are reported as unavailable bindings.

Profile identity is part of compiled artifacts and evidence. A host must refuse an artifact requiring unsupported features. A smaller implementation cannot claim full conformance by treating unsupported effects as no-ops, dropping graph nodes, bypassing policy, or returning fabricated model output.

Three implementation stages are named:

| Profile | Purpose |
|---|---|
| `slice0` | The exact local construction/evaluation subset in Section 24. |
| `application` | General-purpose source, typed errors/data, resources/tasks, complete checking and local memories. |
| `ai-agent` | Application profile plus inference, evidence, planning, durable work, and transport adapters as installed. |

The latter two are milestone labels until their complete conformance suites exist. Evidence must record the actual supported feature set, not rely on the label alone.

## 23. Compatibility and migration

This is a breaking revision. A formatter cannot safely migrate authority behavior by punctuation replacement. Migration must identify old action actor parameters, policy-as-Boolean guards, implicit state reads, model outputs with assumed confidence, and advisory effects that were ever executed.

Old actor arguments become execution-context requirements where appropriate. `? policy p(...)` becomes a required policy attachment; plain Boolean domain guards remain `?`. Shared state reads become explicit snapshot actions. Model results become `Result[Claim[T],InferError]`. Old event sender checks separate origin from executing principal. Existing exact decimal and confidence literals remain meaningful subject to the precise numeric rules above.

The standalone/embedding invariant applies to semantic plans and protected effects. Binary Pulse IR compatibility with draft 0.2 is not promised. Memory records pin language/profile/ABI versions, and incompatible memories require explicit recompilation or a validated adapter.

## 24. Slice 0: exact minimum implementation profile

### 24.1 Supported source and runtime

Slice 0 supports UTF-8 source, the version/module header, comments, exported `fn` and `action` declarations, parameters and results of `String` or `Unit`, explicit action effect sets, string/unit literals, parameter references, string concatenation, calls, and one explicit `return` expression per body. Each source memory exports one callable; helper functions may be present if their call graph is acyclic. `std` is the only implicit namespace and resolves to the locked kernel.

The reference parser/checker bounds expression, additive-chain, and helper-call-graph nesting at 256, reporting diagnostics for inputs beyond that profile limit. The closed kernel effect set is `io.stdout`; unregistered effect declarations are rejected.

Its two kernel operations are `std.text.trim_ascii(String) -> String` and `std.console.write_line(String) -> Unit effects {io.stdout}`. Evaluation is eager, arguments left-to-right, with no implicit I/O during resolution/checking. Runtime faults have stable codes and appear in receipts.

Blueprints support typed inputs, a single `output result`, source/blueprint-memory nodes, data connections, and explicit order edges. Constant `set` bindings, generic types, pattern matching, explicit Pulse source expressions, tasks, agents, sensitive-data annotations, persistent state, network access, inference, and dynamic memory discovery are unsupported and must be rejected.

The blueprint compiler itself constructs and the runtime executes typed Pulse plans. Deferring explicit Pulse surface syntax does not defer the Pulse authority/receipt semantics. Only finite DAG execution is supported. Blueprint memories may be nested; dependency cycles are rejected. No dedicated greeting node or precomputed greeting output is allowed in the implementation.

### 24.2 Store and formats

The local store supports source blobs, memory records, ProgramSpecs, immutable reports, and name/version aliases. All addresses use the canonical encoding/digests in Section 12.1. Source manifests name the module, source files, entry export, and kernel/dependency selectors; the sealer derives types/effects and pins exact identities in a memory record. Source text remains the executable implementation. Kernel descriptors include an implementation build digest as well as their ABI contract, so changing the installed implementation cannot silently preserve execution identity.

The lock format `nexa.lock/0.3` records language/profile, exact kernel digest, source selector/export bindings to memory digests, and dependency closure. A changed root source may reuse the lock only if every selector/export remains bound consistently; a changed selector, missing entry, ABI mismatch, or digest mismatch is a construction error. Input values are never part of a compile-time selector lock. CLI tools select a profile explicitly or use their documented default; the first implementation defaults to `slice0`. The selected profile is persisted in memory records, ProgramSpecs, locks, and evidence.

Application manifests use `nexa.application/0.3`, a requested principal label, capability/resource requests, and declarative operation-specific policy rules. Slice 0 supports `Allow`, `Deny`, `Ask`, and `Abstain` decisions on the installed console operation/stdout resource, encoded as lowercase strings. These rules implement the same decision composition as Section 7; arbitrary source policy functions are deferred. The kernel implementation must match its pinned descriptor before any program operation is invoked.

The launcher binds `@app` to the application's process context and supplies a stdout capability only with `--allow stdout`. A local manifest cannot grant file/network/secret access. An explicit deny dominates an allow; missing/all-abstain policy denies. `Ask` produces a non-executing `AwaitingApproval` outcome, with no interactive approval/resumption in this profile.

### 24.3 Required observable behavior

Required operations are source-memory sealing, explicit resolution and locking, checking, composition, JSON inspection, execution, fixture testing, and sealing a ProgramSpec into a reusable memory. `fmt` and exact signature memory search are follow-on conveniences, not Slice 0 exit criteria.

The evaluator executes loaded AST/Core IR; it MUST NOT switch on fixture memory names or synthesize the expected output. The same runtime must execute a newly sealed source component with different string behavior. A resumed process must load the store/lock without re-resolving aliases.

Required receipts contain schema/profile, program and memory identities, invocation/principal, node path and outcome, derived effects, authorization decisions, and final outcome. Timing may be omitted in this local profile; absent timestamps are explicit. Required inspection exposes nodes, input/result contracts, connections, effect/capability union, memory/source references, and unresolved requirements.

### 24.4 Acceptance evidence

The minimum evidence set includes successful source and composed execution; rejected type/port/cycle/effect errors; denial without stdout permission; deny-dominant policy; inert `Ask`; detection of changed bytes under an existing digest; locked behavior after an alias moves; equivalent output after sealing/reusing the whole graph; and execution after a fresh process loads the store.

The fixture greeting is known good only for those checks and the named text inputs under `slice0`. It makes no inference, throughput, persistence, or general concurrency claim. The companion plan specifies the repository structure, milestones, fixtures, and command sequence that produce this evidence.

## 25. Design decisions and future work

This revision settles immutable memory identity, explicit locks, nominal port typing, pure-function boundaries, authority at protected effects, first-match clauses within policies, deny dominance across policies, blueprint DAG semantics, and evidence separate from executable identity. Implementers should not reinterpret these as open questions.

Future work includes broader source-ergonomics experiments, a complete protocol schema package, formal verification integrations, distributed registry protocols, richer constraint solving, zero-copy ownership profiles, native targets, and the web inspector. A future solver must expose its assumptions and generated adapters. A future syntax change must be evaluated on whole development tasks, including human review and agent inspection costs.

The first implementation should answer a narrow question: can a short, typed construction request assemble real source memories, produce an inspectable program, enforce its one effect boundary, and turn that program into a reusable memory? Its acceptance suite makes that question observable.

## 26. Sources and rationale

This specification is based on the supplied Nexa language draft 0.1, the supplied HTML draft 0.2, and the subsequent design discussion. The earlier documents are historical design inputs; this revision's explicit semantics take precedence where they differ.

The small lexical experiment in the discussion compared four fragments with two tokenizers. Replacing `is` with `:` did not reduce tokens in those fragments; combined list/annotation/return changes saved about 2.8%. That experiment supports retaining readable syntax while measuring broader task costs. It is not evidence of performance or comprehension for this proposed language.

The reference implementation exercises Slice 0 through source/runtime, integrity, composition, reuse, inspection, and subprocess acceptance tests. See the [reproduction guide](slice0-demo.md). Full-language conformance, human-comprehension results, throughput benchmarks, and model evaluations are not claimed.
