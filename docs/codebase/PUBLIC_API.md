# Public API And Stability

Updated: 2026-07-23. Current release: `0.1.49`.

This document defines compatibility-sensitive API boundaries for the published
mobench crates. Use it during semver reviews, docs.rs cleanup, feature-flag
changes, and release readiness checks.

## Published Crates

### `mobench-sdk`

Stable public surface:

- `mobench_sdk::benchmark`
- `mobench_sdk::debug_benchmarks`
- `mobench_sdk::BenchmarkBuilder`
- `mobench_sdk::run_benchmark`
- `mobench_sdk::discover_benchmarks`
- `mobench_sdk::find_benchmark`
- `mobench_sdk::list_benchmark_names`
- `mobench_sdk::black_box`
- `mobench_sdk::export_native_c_abi!()`
- `mobench_sdk::MobenchBuf`
- `mobench_sdk::{BenchError, Target, FfiBackend}`
- `mobench_sdk::{InitConfig, BuildConfig, BuildProfile}`
- `mobench_sdk::{NativeLibraryArtifact, BuildResult}`
- `mobench_sdk::timing::{BenchSpec, BenchSample, BenchReport, BenchSummary}`
- `mobench_sdk::timing::{SemanticPhase, HarnessTimelineSpan, TimingError}`
- `mobench_sdk::timing::{profile_phase, run_closure}`
- `mobench_sdk::builders::{AndroidBuilder, IosBuilder, SigningMethod}`

Supported lower-level public surface:

- `mobench_sdk::ffi`
- `mobench_sdk::uniffi_types`
- `mobench_sdk::native_c_abi`
- `mobench_sdk::codegen`
- `mobench_sdk::builders::common`

Generated runners, examples, and advanced integrations use these lower-level
modules. Breaking changes should be paired with release-note migration guidance.

### `mobench`

Stable public surface:

- `mobench` and `cargo-mobench` binaries.
- CLI flags and subcommands documented by `--help`.
- Programmatic integration types: `RunRequest`, `RunResult`,
  `DeviceSelection`, `Report`, and `run_request`.
- Public config types in `mobench::config`.
- JSON, Markdown, CSV, profiling, trace-event, and schema contracts documented
  under `docs/schemas/` and `docs/guides/`.

The CLI may use `anyhow::Result` internally. User-facing errors should carry
actionable context before crossing the command boundary.

### `mobench-macros`

Stable public surface:

- `#[benchmark]`
- `setup`, `teardown`, and `per_iteration` attribute syntax.
- Compile-time signature validation diagnostics.

Most users consume this macro through `mobench_sdk::benchmark`.

## Feature Flags

`mobench-sdk` feature groups:

- `default`: enables `full`.
- `full`: enables `registry`, `builders`, and `codegen`.
- `registry`: enables `mobench-macros`, `inventory`, benchmark discovery, and
  runtime execution.
- `builders`: enables mobile build automation and depends on `codegen`.
- `codegen`: enables generated project and template support.
- `runner-only`: minimal timing-only mode for mobile binaries.

Feature policy:

- Keep `runner-only` free of build tooling dependencies.
- Prefer `registry` for benchmark crates that need discovery but not builders.
- Keep default features convenient for normal SDK users.
- Add narrower optional features only when they reduce dependency footprint or
  clarify platform support.
- Document feature behavior changes in `RELEASE_NOTES.md`.

## Serialized Contracts

Compatibility-sensitive serialized values:

- `Target`: `android`, `ios`, `both`.
- `FfiBackend`: `uniffi`, `native-c-abi`.
- CI output files: `summary.json`, `summary.md`, `results.csv`, optional
  `plots/*.svg`.
- Resource columns: `cpu_total_ms`, `cpu_median_ms`, `peak_memory_kb`,
  `peak_memory_growth_kb`, `process_peak_memory_kb`.
- Native C ABI symbols: `mobench_run_benchmark_json`, `mobench_free_buf`,
  `mobench_last_error_message`.
- Profile manifest sections: `native_capture`, `semantic_profile`,
  `capture_metadata`.

Schemas live in `docs/schemas/`.

## Error Handling Boundary

- SDK APIs should expose typed errors through `BenchError` or `TimingError`.
- CLI orchestration may use `anyhow::Result` internally.
- Generated FFI surfaces should convert errors into FFI-safe values.
- New reusable SDK APIs should avoid exposing `anyhow::Error` unless the API is
  explicitly CLI-only or transitional.

## Semver Policy

Before `1.0`, mobench still treats these surfaces as compatibility-sensitive:

- Benchmark macro syntax used by downstream crates.
- `mobench-sdk` top-level re-exports.
- `FfiBackend` serialized values.
- Native C ABI exported symbol names.
- Serialized benchmark and CI output fields.
- Documented CLI flags used in CI.
- Generated template inputs and output artifact names.

Allowed in a minor release:

- Adding fields when deserializers tolerate them.
- Adding CLI flags with compatible defaults.
- Adding optional outputs.
- Adding non-default crate features.
- Improving diagnostics without changing machine-readable contracts.

Requires migration notes:

- Removing or renaming public Rust items.
- Changing default feature composition.
- Changing serialized field names or units.
- Changing generated template paths consumed by users.
- Changing CLI defaults that affect output location or benchmark behavior.

## MSRV

Workspace MSRV is Rust 1.85, the first stable release with Rust 2024 edition
support. Crates inherit through `workspace.package.rust-version`.

If a dependency or language feature requires raising MSRV:

- Update `workspace.package.rust-version`.
- Update this document.
- Mention the change in `RELEASE_NOTES.md`.
- Verify the quality workflow on the new stable toolchain.

## Release Readiness Checks

Run before publishing:

```bash
cargo fmt --all --check
cargo test --workspace --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --all-features --no-deps
cargo publish --dry-run -p mobench-macros
cargo publish --dry-run -p mobench-sdk
cargo publish --dry-run -p mobench
```

Publish order is `mobench-macros`, then `mobench-sdk`, then `mobench`.
Dependent dry-runs should happen after the dependency version is available from
the crates.io registry index.
