# Integrations

Updated: 2026-07-15. Release line: `0.1.43`.

## BrowserStack

Purpose:

- Benchmark execution on real Android and iOS devices.
- Device inventory lookup and validation.
- Session artifact download.
- Optional timing/resource metric enrichment.

Implementation:

- Client code: `crates/mobench/src/browserstack.rs`.
- Auth sources: config, `BROWSERSTACK_USERNAME`, `BROWSERSTACK_ACCESS_KEY`,
  optional `BROWSERSTACK_PROJECT`, and `.env.local`.
- Device resolution: `cargo mobench devices` and `cargo mobench devices
  resolve`.
- Fetching: `cargo mobench fetch`, `run --fetch`, and `ci run --fetch`.

Supported BrowserStack flows:

- Android Espresso benchmark runs.
- iOS XCUITest benchmark runs.
- Device listing, validation, and deterministic matrix/profile resolution.
- Artifact fetch after benchmark runs.
- Timing/resource metric normalization when data is available.

Explicitly unsupported:

- BrowserStack native profiling through `profile run`.
- Retrievable BrowserStack flamegraph/native stack artifacts.

## Generated Runner Backends

`mobench.toml` selects the generated runner backend:

```toml
[project]
ffi_backend = "uniffi" # or "native-c-abi"
```

- `uniffi`: compatibility backend using generated Kotlin/Swift bindings.
- `native-c-abi`: direct JSON C ABI backend using
  `mobench_sdk::export_native_c_abi!()`.

Both backends emit benchmark JSON for CLI parsing and CI normalization.

## Local Native Profiling

Android integration:

- Build a profileable app.
- Launch through `adb`.
- Capture with `simpleperf`.
- Symbolize native frames with NDK `llvm-addr2line`.
- Render full/focused flamegraphs and `flamegraph.html`.

iOS integration:

- Build and install simulator app.
- Launch with profiling-specific environment/arguments.
- Capture with macOS `sample`.
- Derive focused/full folded stacks.
- Render full/focused flamegraphs and `flamegraph.html`.

`rust-tracing` currently provides a planned manifest/trace contract rather than
real native capture.

## GitHub Actions

Current workflow families:

- `rust.yml`: Rust quality checks.
- `compile-gate.yml`: compile-only gate.
- `mobile-bench.yml`: dispatchable benchmark workflow.
- `reusable-bench.yml`: reusable benchmark workflow.
- `mobile-bench-pr-auto.yml`: automatic PR benchmark dispatch.
- `mobile-bench-pr-command.yml`: command-triggered PR benchmark dispatch.
- `reusable-pr-auto.yml` and `reusable-pr-command.yml`: reusable PR workflow
  pieces.
- `mobile-bench-plot-fixtures.yml`: plot fixture validation.
- `mobile-bench-profile-selftest.yml`: local profile artifact validation.
- `mobile-bench-selftest.yml`: benchmark self-test workflow.
- `mobile-bench-action-example.yml`: action usage example.

Primary artifact contracts:

- Benchmark CI: `summary.json`, `summary.md`, `results.csv`, optional plots.
- BrowserStack fetch: session JSON, logs, and available provider artifacts.
- Profiling self-test: profile manifest, summary, folded stacks, flamegraphs,
  and viewer artifacts.

## Local Configuration And Credentials

Resolution order:

1. Explicit CLI flags.
2. Explicit `--config` path.
3. Discovered `mobench.toml`.
4. Cargo workspace metadata.
5. Git root.
6. Legacy fallback paths.

BrowserStack credentials additionally load from environment variables and
`.env.local`.
