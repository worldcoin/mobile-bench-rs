# Changelog

All notable user-facing changes to `mobench`, `mobench-sdk`, and
`mobench-macros` are tracked here by release. See `RELEASE_NOTES.md` for the
longer integration-oriented release notes and support status.

## Unreleased

No user-facing unreleased changes yet.

## v0.1.43 - 2026-07-05

### Added

- Added `cargo mobench ci merge-split-runs` for CI workflows that run each
  measured sample as its own `cargo mobench ci run` invocation.
- Added split-run merge documentation for long or fragile BrowserStack lanes.

### Changed

- Split-run merging writes the same `summary.json`, `summary.md`, and
  `results.csv` contract used by normal `mobench ci run` output.
- Merged summaries recompute measured timing statistics and resource columns so
  existing report, plot, PR comment, and comparison tooling can consume them
  unchanged.

### Validation

- Merge inputs are rejected unless they contain exactly one device, exactly one
  benchmark result, the requested benchmark function, the requested device, and
  exactly the requested measured sample count.

## v0.1.42 - 2026-06-29

### Fixed

- Propagated config-selected `native-c-abi` backends through Android builds, iOS
  builds, CI runs, and iOS BrowserStack packaging.
- Prevented CI flows from rebuilding native C ABI projects with the default
  UniFFI backend.

### Changed

- Kept BrowserStack log/result extraction compatible across `uniffi` and
  `native-c-abi` generated runners.
- Refreshed documentation for the backend matrix, profiling artifact layout, and
  CI output contract.

## v0.1.41 - 2026-05-14

### Added

- Added `[project].ffi_backend` with the default `uniffi` backend and direct
  `native-c-abi` JSON runner support.
- Added `mobench_sdk::export_native_c_abi!()` and native C ABI exports for
  registry-based benchmark crates.
- Added generated native C ABI runner templates for Android and iOS.
- Added native C ABI headers to generated iOS frameworks when that backend is
  selected.

## v0.1.37 - 2026-04-27

### Added

- Added `cargo mobench profile run --trace-events-output <path>` for
  machine-readable harness trace/event JSON.
- Added the `mobench-sdk` `registry` feature for benchmark macro registration,
  inventory discovery, and runtime execution without builder/template
  dependencies.
- Added property-test coverage for run config device matrix parsing.

### Changed

- Narrowed generated FFI wrapper example crates to the `registry` feature
  instead of the full SDK build-tooling feature set.

## v0.1.36 - 2026-04-27

### Added

- Added production-readiness documentation for public APIs, semver boundaries,
  feature flags, MSRV, release checks, examples, and diagrams.
- Added Rust quality CI covering rustfmt, clippy, rustdoc, tests, and manual
  publish dry-runs.
- Added opt-in structured CLI tracing through `--verbose` or `MOBENCH_LOG`.
- Added explicit `doctor` MSRV checks.
- Added host-only fixture contract coverage for stable Markdown and CSV
  rendering.

### Fixed

- Hardened clean first-run spec embedding for generated Android and iOS
  projects.
- Restricted authenticated BrowserStack artifact downloads to BrowserStack HTTPS
  hosts.
- Restored config-file runs without duplicate `--target` or `--function` flags
  while preserving CLI-over-config precedence.
- Tightened generated mobile template compatibility around minimal UniFFI report
  types.
- Added compile-fail coverage for async benchmark functions and setup/teardown
  error behavior.

## v0.1.35 - 2026-04-24

### Added

- Added iOS benchmark app process peak memory reporting using Mach `task_info`.
- Added Android foreground service type metadata required by newer Android
  platform rules.

### Changed

- Marked iOS process peak resources with
  `memory_process = "benchmark_app"` to match the Android summary contract while
  reflecting iOS app-process execution.

## v0.1.34 - 2026-04-23

### Added

- Added Mobile Bench workflow branch-pinned validation.

## v0.1.32 and earlier

Historical test builds unless explicitly noted in package metadata. Earlier
releases covered the initial CI contract, BrowserStack orchestration,
setup/teardown macro support, generated mobile templates, device matrices, and
consolidation of old `mobench-runner` functionality into `mobench-sdk`.
