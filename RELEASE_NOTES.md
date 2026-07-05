# Release Notes

`mobench`, `mobench-sdk`, and `mobench-macros` were published rapidly during
bring-up. Only the current release line should be treated as supported. Earlier
crates.io publishes are retained for auditability, but should not be used for
new integrations unless explicitly noted.

Crates.io release pages:

- [mobench](https://crates.io/crates/mobench)
- [mobench-sdk](https://crates.io/crates/mobench-sdk)
- [mobench-macros](https://crates.io/crates/mobench-macros)

## Unreleased

No user-facing unreleased changes yet.

## v0.1.43

Status: current supported release.

Publication date: 2026-07-05.

### CI split-run merging

- Added `cargo mobench ci merge-split-runs` to merge one-measured-sample CI
  summaries back into standard `summary.json`, `summary.md`, and `results.csv`
  artifacts.
- Validates the requested benchmark function, requested device, one benchmark
  per input summary, consistent target, and exact measured sample count before
  writing merged outputs.
- Recomputes `min_ns`, `max_ns`, `mean_ns`, `median_ns`, `p95_ns`, `samples_ns`,
  and benchmark resource columns so downstream report/plot consumers can use the
  merged result like normal `ci run` output.
- Documented the workflow for long or fragile BrowserStack lanes that need to
  run each measured sample as its own CI invocation.

## v0.1.42

Status: superseded by `v0.1.43`.

Publication date: 2026-06-29.

### Native C ABI release hardening

- Fixed `cargo mobench ci run` and `mobench run` build helpers so
  config-selected `native-c-abi` backends propagate through Android builds, iOS
  builds, CI runs, and iOS BrowserStack packaging.
- Prevented CI and run flows from rebuilding native C ABI projects with the
  default UniFFI backend.
- Kept BrowserStack log/result extraction compatible across `uniffi` and
 `native-c-abi` generated runners.
- Documented the merged `boltffi` generated runner backend alongside
 `uniffi` and `native-c-abi`.
- Refreshed the root README, crate READMEs, release docs, and Mermaid diagrams
  for the current backend matrix, profiling artifact layout, and CI output
  contract.

## v0.1.41

Status: superseded by `v0.1.42`.

- Added `[project].ffi_backend` with `uniffi` as the compatibility default and
  `native-c-abi` for direct mobench JSON C ABI benchmark runners.
- Added `mobench_sdk::export_native_c_abi!()` and `MobenchBuf` so
  registry-based benchmark crates can export:
  - `mobench_run_benchmark_json`
  - `mobench_free_buf`
  - `mobench_last_error_message`
- Updated Android and iOS builders to branch on selected FFI backend, skip
  UniFFI binding generation for `native-c-abi`, and generate native JSON C ABI
  runner templates.
- Added native C ABI headers to generated iOS frameworks when the backend is
  selected.

## v0.1.37

Status: superseded by `v0.1.41`.

- Added `cargo mobench profile run --trace-events-output <path>` for downstream
  consumers that need machine-readable harness trace/event JSON.
- Added the `mobench-sdk` `registry` feature for benchmark macro registration,
  inventory discovery, and runtime execution without builder/template
  dependencies.
- Moved generated FFI wrapper crates and example benchmark crates to the
  narrower `registry` feature instead of the full SDK build-tooling feature set.
- Added property-test coverage for run config and device matrix parsing.

## v0.1.36

Status: superseded by `v0.1.37`.

- Added production-readiness documentation for public API boundaries, semver
  expectations, feature flags, MSRV, release checks, examples, and launch
  diagrams.
- Added Rust quality CI covering rustfmt, clippy, rustdoc, tests, and
  manually-triggered publish dry-runs.
- Added opt-in structured CLI tracing through `--verbose` or `MOBENCH_LOG`, plus
  explicit `doctor` MSRV checks.
- Added host-only fixture contract coverage and stable Markdown/CSV rendering.
- Hardened clean first-run spec embedding for generated Android and iOS
  projects.
- Restricted authenticated BrowserStack artifact downloads to BrowserStack HTTPS
  hosts.
- Restored config-file runs without duplicate `--target` / `--function` flags
  and preserved CLI-over-config precedence.
- Tightened generated mobile template compatibility for minimal UniFFI report
  types.
- Added compile-fail coverage for async benchmark functions and setup/teardown
  error behavior.

## v0.1.35

Status: superseded by `v0.1.36`.

- Added iOS benchmark app process peak memory reporting using Mach `task_info`.
- Marked iOS process peak resources with `memory_process = "benchmark_app"` to
  match the Android summary contract while reflecting iOS app-process execution.
- Added Android foreground service type metadata required by newer Android
  platform rules.

## v0.1.34

Status: superseded by `v0.1.35`.

- Rendered one SVG plot per benchmark function in the `Device Comparison Plots`
  summary section.
- Standardized benchmark-scoped resource columns in `results.csv`.
- Added BrowserStack metric normalization documentation.

## v0.1.33

Status: superseded by `v0.1.34`.

- Measured benchmark CPU time as process user-plus-kernel time across all
  threads.
- Reworked rendered CI summaries into one table with wall mean, wall total, CPU
  median, CPU total, CPU-to-wall ratio, and peak memory columns.
- Exposed `mobench_ref` and `mobench_version` on the manual Mobile Bench
  workflow for branch-pinned validation.

## v0.1.32 and Earlier

Status: historical test builds unless explicitly noted in their package
metadata. Do not use for new integrations.

Earlier releases covered the initial CI contract, BrowserStack orchestration,
setup/teardown macro support, generated mobile templates, device matrices, and
the consolidation of the old `mobench-runner` functionality into
`mobench-sdk`.

## Published Version History

| Version | Published | Published crates | Status |
| --- | --- | --- | --- |
| `v0.1.43` | 2026-07-05 | `mobench 0.1.43`, `mobench-sdk 0.1.43`, `mobench-macros 0.1.43` | Current supported release |
| `v0.1.42` | 2026-06-29 | `mobench 0.1.42`, `mobench-sdk 0.1.42`, `mobench-macros 0.1.42` | Superseded by `v0.1.43` |
| `v0.1.41` | 2026-05-14 | `mobench 0.1.41`, `mobench-sdk 0.1.41`, `mobench-macros 0.1.41` | Superseded by `v0.1.42` |
| `v0.1.37` | 2026-04-27 | `mobench 0.1.37`, `mobench-sdk 0.1.37`, `mobench-macros 0.1.37` | Superseded by `v0.1.41` |
| `v0.1.36` | 2026-04-27 | `mobench 0.1.36`, `mobench-sdk 0.1.36`, `mobench-macros 0.1.36` | Superseded by `v0.1.37` |
| `v0.1.35` | 2026-04-24 | `mobench 0.1.35`, `mobench-sdk 0.1.35`, `mobench-macros 0.1.35` | Superseded by `v0.1.36` |
| `v0.1.34` | 2026-04-23 | `mobench 0.1.34`, `mobench-sdk 0.1.34`, `mobench-macros 0.1.34` | Superseded by `v0.1.35` |
| `v0.1.33` | 2026-04-17 | `mobench 0.1.33`, `mobench-sdk 0.1.33`, `mobench-macros 0.1.33` | Superseded by `v0.1.34` |
| `v0.1.32` and earlier | 2026-01 to 2026-04 | See crates.io package history | Historical test builds |
