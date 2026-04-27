# Public API And Stability

Updated: 2026-04-26

## Purpose

This document defines the public API boundaries for the published mobench
crates. It is the starting point for semver reviews, docs.rs cleanup, feature
flag changes, and release readiness checks.

## Published Crates

### `mobench-sdk`

Primary audience: library adopters and generated mobile runners.

Stable public surface:
- `mobench_sdk::benchmark`
- `mobench_sdk::BenchmarkBuilder`
- `mobench_sdk::run_benchmark`
- `mobench_sdk::discover_benchmarks`
- `mobench_sdk::find_benchmark`
- `mobench_sdk::list_benchmark_names`
- `mobench_sdk::black_box`
- `mobench_sdk::timing::{BenchSpec, BenchSample, BenchReport, BenchSummary}`
- `mobench_sdk::timing::{SemanticPhase, HarnessTimelineSpan, TimingError}`
- `mobench_sdk::timing::{profile_phase, run_closure}`
- `mobench_sdk::{BenchError, Target, InitConfig, BuildConfig, BuildProfile}`
- `mobench_sdk::{NativeLibraryArtifact, BuildResult}`
- `mobench_sdk::builders::{AndroidBuilder, IosBuilder, SigningMethod}`

Supported but lower-level public surface:
- `mobench_sdk::ffi`
- `mobench_sdk::uniffi_types`
- `mobench_sdk::codegen`
- `mobench_sdk::builders::common`

These modules are public because generated runners, examples, or advanced
integrations use them. They should remain documented, but breaking changes here
can be considered when release notes include migration guidance.

### `mobench`

Primary audience: CLI users and CI integrations.

Stable public surface:
- the `mobench` and `cargo-mobench` binaries
- CLI flags and subcommands documented by `--help`
- JSON, Markdown, CSV, and profiling artifact contracts documented under
  `docs/schemas/`, `docs/guides/`, and `README.md`
- programmatic types exported from `crates/mobench/src/lib.rs`:
  - `DeviceSelection`
  - `RunRequest`
  - `RunResult`
  - `Report`
  - `run_request`

The CLI may continue using `anyhow` internally. User-facing failures should add
actionable context before crossing the command boundary.

### `mobench-macros`

Primary audience: benchmark authors through `mobench-sdk`.

Stable public surface:
- `#[benchmark]`
- supported attributes documented in the macro rustdoc, including setup,
  teardown, and per-iteration setup behavior

## Feature Flags

`mobench-sdk` currently exposes two features:

- `full`: default feature. Enables the benchmark macro, inventory registry,
  builders, embedded templates, codegen, and TOML-backed build automation.
- `runner-only`: minimal mobile-runtime mode for generated/mobile benchmark
  binaries where build automation is not needed.

Feature policy:
- keep `runner-only` free of build tooling dependencies
- keep default features convenient for normal SDK users
- add narrower optional features only when they measurably reduce dependency
  footprint or clarify platform support
- document any feature behavior change in release notes

## Error Handling Boundary

- SDK APIs should expose typed errors via `BenchError` or `TimingError`.
- CLI orchestration may use `anyhow::Result` internally and at CLI entrypoints.
- Generated FFI surfaces should convert errors into FFI-safe error enums.
- New reusable SDK APIs should not expose `anyhow::Error` unless the API is
  explicitly CLI-only or transitional.

## Semver Policy

Before `1.0`, mobench still treats the following as compatibility-sensitive:

- benchmark macro syntax used by downstream crates
- `mobench-sdk` top-level re-exports
- serialized benchmark and CI output fields
- documented CLI flags used in CI
- generated template inputs and output artifact names

Allowed in a minor release:
- adding fields when deserializers tolerate them
- adding CLI flags with defaults
- adding new optional outputs
- adding non-default crate features
- improving diagnostics without changing machine-readable contracts

Requires migration notes:
- removing or renaming public Rust items
- changing default feature composition
- changing serialized field names or units
- changing generated template paths consumed by users
- changing CLI defaults that affect output location or benchmark behavior

## MSRV

The workspace MSRV is Rust 1.85, the first stable release with Rust 2024
edition support. Crates inherit this through `workspace.package.rust-version`.

If a dependency or language feature requires raising MSRV:
- update `workspace.package.rust-version`
- update this document
- mention the change in `RELEASE_NOTES.md`
- verify the quality workflow still passes on the new stable toolchain

## Release Readiness Checks

Run these before publishing the published crates:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --all-features --no-deps
cargo test --workspace --locked
cargo publish --dry-run -p mobench-macros
cargo publish --dry-run -p mobench-sdk
cargo publish --dry-run -p mobench
```

Publish order is `mobench-macros`, `mobench-sdk`, then `mobench`.
