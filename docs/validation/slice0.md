# Slice 0 validation record

The local reference slice was validated against implementation commit [`0b9250b`](https://github.com/haha-systems/nexa/commit/0b9250bdc936828d7edf16f4e58f0504c0564e5a). The subsequent validation-document commit does not change implementation inputs.

## Checks performed

- Formatting and all-target Clippy with warnings denied passed.
- `cargo test --locked` passed all 72 tests.
- All 12 generated JSON Schemas and example JSON documents validated.
- Documentation links and anchors checked.
- A fresh Git clone built with the pinned toolchain and ran every shell section in the [demo](../slice0-demo.md).
- Source and typed-request construction produced byte-identical ProgramSpecs.
- Original and saved/nested programs succeeded. Both forms denied stdout without its host grant, with no application output.
- All six greeting fixtures passed and the source checkout remained clean after generated outputs.

## Captured fixture evidence

The [actual CLI report](slice0-local-report.json) retains each expectation, observed stdout bytes/outcome, permission setup, and environment. Its subject is `sha256:f1c01cb16b5bfa357df17aa126593c9c86a10f809826a9711865afa7fc23a68e` and its pinned kernel is `sha256:2eca4ccf13bff062b474aafb12b18057db636e182542fa6b8fd1be89798d8301`.

These identities apply to the named implementation/build configuration. They may differ on another target or with different build inputs; the workflow reseals its own memories explicitly. The report is fixture evidence, not a universal correctness, model, or throughput claim.

Reports and their references are content-addressed separately from executable objects. This pretty-printed copy preserves the report's semantic contents; canonical document hashing determines its semantic identity.
