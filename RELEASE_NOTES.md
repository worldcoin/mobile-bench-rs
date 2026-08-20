# Release Notes

The 0.2 release publishes the CLI, SDK, macro, and rewrite foundation crates
together. Treat only the current release line as supported for new integrations
unless an older release is explicitly called out. For a concise release-by-
release change list, see `CHANGELOG.md`.

Crates.io release pages:

- [mobench](https://crates.io/crates/mobench)
- [mobench-sdk](https://crates.io/crates/mobench-sdk)
- [mobench-macros](https://crates.io/crates/mobench-macros)
- [mobench-runtime](https://crates.io/crates/mobench-runtime)
- [mobench-domain](https://crates.io/crates/mobench-domain)
- [mobench-process](https://crates.io/crates/mobench-process)
- [mobench-artifacts](https://crates.io/crates/mobench-artifacts)
- [mobench-provider](https://crates.io/crates/mobench-provider)
- [mobench-report](https://crates.io/crates/mobench-report)

## v0.2.1 - 2026-08-20

Status: release candidate.

Mobench 0.2.1 adds CPU-topology and effective-utilization diagnostics to generated
Android and iOS runners. Reports now distinguish the device's logical processor count
from the processors available to the benchmark process, record an explicit
`RAYON_NUM_THREADS` environment setting when present, and show the median effective
core count calculated from process CPU time divided by wall time.

The release also includes `sample_fns::parallel_cpu_saturation`, a controlled
multicore fixture intended to verify that a runner can schedule concurrent CPU work.
Comparing it with an application fixture makes serial application code distinguishable
from provider affinity or quota constraints.

## v0.2.0 - 2026-07-31

Status: current supported release.

Mobench 0.2 is a clean rewrite that preserves the v0.1 feature contract while
splitting the implementation into explicit, testable workspace boundaries. All
nine workspace crates are published at 0.2.0 so downstream users can depend on
the CLI, SDK, or a focused runtime/reporting boundary independently.

### Benchmark authoring and execution

- Preserved `#[benchmark]` registration, compile-time signature validation,
  inventory discovery, setup/teardown, per-iteration setup, warmup, measured
  samples, and custom scalar metrics.
- Preserved host-only, local Android/iOS, and BrowserStack execution through the
  `mobench` CLI and its programmatic `RunRequest`/`RunResult` API.
- Preserved config-first project resolution, device profiles and structured
  device matrices, per-platform function selection, custom toolchains/targets,
  release builds, split-sample merging, fetch/summary commands, and doctor/check
  prerequisite diagnostics.

### Generated runners and FFI

- Preserved generated Android and iOS projects with synchronized editable and
  embedded templates, UniFFI compatibility, BoltFFI compatibility, and the
  direct `native-c-abi` JSON runner backend.
- Added Android-native-width C ABI `usize` fields for 32-bit devices, panic
  diagnostics, custom metrics, worker-death watchdogs, fail-closed result
  handoff, and reliable iOS heartbeat/accessibility result transport.
- Preserved strict requested/observed report identity and run-scoped benchmark
  configuration embedding in generated mobile artifacts.

### Browser and WASM

- Added browser-safe timing and resource behavior, `runBenchmarkJson`, a
  version-matched `wasm-bindgen` builder, `build --target web`, and `run-web`.
- Added a dedicated module worker so synchronous proving does not block browser
  polling, plus synchronized editable and embedded web templates.
- Added direct W3C BrowserStack Automate transport with BrowserStack Local
  lifecycle, bounded connect/request/session timeouts, cleanup, credential
  redaction, and deterministic artifact handling.
- Added a release gate for both Rust-source downstream WASM adapters and the
  published `@worldcoin/provekit` browser SDK. The npm lane proves and verifies
  through the public SDK API and rejects a tampered proof.

### Security, artifacts, reports, and profiling

- Split reusable CI into secretless prepare jobs and credentialed prebuilt-only
  execution. Handoffs are enumerated, hashed, immutable, and checked against
  the exact candidate SHA; caller hooks, dependencies, toolchains, and FFI
  customization never run with provider credentials.
- Added explicit runtime, domain, process, provider, report, and artifact
  crates with bounded counts, strict v2 report envelopes, subprocess
  supervision, context-safe Markdown/CSV/GitHub rendering, atomic immutable
  publication, manifests, leases, recovery, latest snapshots, and retention.
- Preserved JSON, Markdown, CSV, SVG plot, PR/check-run, trace-event, and local
  native profiling outputs, including Android `simpleperf`, iOS simulator
  `sample`, symbol caches, flamegraphs, semantic phases, and profile diffs.
  BrowserStack native stack/flamegraph profiling remains explicitly unsupported.

### Compatibility and acceptance

- Feature parity is tracked against v0.1.49, including the v0.1.40-v0.1.49
  additions, native Android reliability fixes, FFI backends, reporting/resource
  contracts, profiling, secure reusable workflows, and BrowserStack behavior.
- The exact candidate passed the full 40-job release gate in run
  [30652763040](https://github.com/worldcoin/mobile-bench-rs/actions/runs/30652763040):
  two Android and two iOS devices for ProveKit and world-id-protocol, plus
  macOS Safari, Windows Chrome, iOS Safari, and Android Chrome for both
  downstream WASM lanes and the `@worldcoin/provekit` npm lane.
- Local acceptance includes locked workspace tests, formatting, Clippy,
  workflow security tests, actionlint, fresh pinned downstream WASM builds, and
  template synchronization checks. See
  [`docs/0.2-feature-parity-checklist.md`](docs/0.2-feature-parity-checklist.md).

## v0.1.43

Status: current supported release.

Publication date: 2026-07-05.

### CI Split-Run Merging

- Added `cargo mobench ci merge-split-runs` for CI workflows that run each
  measured sample as a separate `cargo mobench ci run` invocation.
- Merges `sample-*/summary.json` inputs back into standard `summary.json`,
  `summary.md`, and `results.csv` outputs.
- Validates the requested benchmark function, requested device, one benchmark
  per input summary, one device per input summary, and exact measured sample
  count before writing merged outputs.
- Recomputes `samples_ns`, `min_ns`, `max_ns`, `mean_ns`, `median_ns`, `p95_ns`,
  and resource columns so existing report, plot, PR comment, and comparison
  tooling can consume merged results unchanged.
- Documents the split-sample workflow for long or fragile BrowserStack lanes
  that need to run each measured sample as its own provider invocation.

## v0.1.42

Status: superseded by `v0.1.43`.

Publication date: 2026-06-29.

### Native C ABI Release Hardening

- Fixed `cargo mobench ci run` and `mobench run` build helpers so
  config-selected `native-c-abi` backends propagate through Android builds, iOS
  builds, CI runs, and iOS BrowserStack packaging.
- Prevented CI run flows from rebuilding native C ABI projects with the default
  UniFFI backend.
- Kept BrowserStack log/result extraction compatible across `uniffi` and
  `native-c-abi` generated runners.
- Documented the merged `boltffi` runner backend alongside `uniffi` and
  `native-c-abi`.
- Refreshed the root README, crate READMEs, release docs, and Mermaid diagrams
  for the current backend matrix, profiling artifact layout, and CI output
  contract.

## v0.1.41

Status: superseded by `v0.1.42`.

- Added `[project].ffi_backend` with `uniffi` as the compatibility default and
  `native-c-abi` as the direct mobench JSON C ABI benchmark runner backend.
- Added `mobench_sdk::export_native_c_abi!()` and `MobenchBuf` so
  registry-based benchmark crates export:
  - `mobench_run_benchmark_json`
  - `mobench_free_buf`
  - `mobench_last_error_message`
- Updated Android and iOS builders to branch on the selected FFI backend, skip
  UniFFI binding generation for `native-c-abi`, and generate native JSON C ABI
  runner templates.
- Added native C ABI headers to generated iOS frameworks when that backend is
  selected.

## v0.1.37

Status: superseded by `v0.1.41`.

- Added `cargo mobench profile run --trace-events-output <path>` for downstream
  consumers that need machine-readable harness trace/event JSON.
- Added the `mobench-sdk` `registry` feature for benchmark macro registration,
  inventory discovery, and runtime execution without builder/template
  dependencies.
- Moved generated FFI wrapper example benchmark crates to the narrower
  `registry` feature instead of the full SDK build-tooling feature set.
- Added property-test coverage for run config device matrix parsing.

## v0.1.36

Status: superseded by `v0.1.37`.

- Added production-readiness documentation for public API boundaries, semver
  expectations, feature flags, MSRV, release checks, examples, and launch
  diagrams.
- Added Rust quality CI covering rustfmt, clippy, rustdoc, tests, and
  manually-triggered publish dry-runs.
- Added opt-in structured CLI tracing through `--verbose` or `MOBENCH_LOG`, plus
  explicit `doctor` MSRV checks.
- Added host-only fixture contract coverage for stable Markdown and CSV
  rendering.
- Hardened clean first-run spec embedding for generated Android and iOS
  projects.
- Restricted authenticated BrowserStack artifact downloads to BrowserStack HTTPS
  hosts.
- Restored config-file runs without duplicate `--target` / `--function` flags
  while preserving CLI-over-config precedence.
- Tightened generated mobile template compatibility around minimal UniFFI report
  types.
- Added compile-fail coverage for async benchmark functions and setup/teardown
  error behavior.

## v0.1.35

Status: superseded by `v0.1.36`.

- Added iOS benchmark app process peak memory reporting using Mach `task_info`.
- Marked iOS process peak resources as `memory_process = "benchmark_app"` to
  match the Android summary contract while reflecting iOS app-process execution.
- Added Android foreground service type metadata required by newer Android
  platform rules.

## v0.1.34

Status: superseded by `v0.1.35`.

- Rendered one SVG plot per benchmark function in the
  `Device Comparison Plots` summary section.
- Standardized benchmark-scoped resource columns in `results.csv`.
- Added BrowserStack metric normalization documentation.

## v0.1.33

Status: superseded by `v0.1.34`.

- Measured benchmark CPU time as process user-plus-kernel time across all
  threads.
- Reworked rendered CI summaries into one table covering wall mean, wall total,
  CPU median, CPU total, CPU-to-wall ratio, and peak memory columns.
- Exposed `mobench_ref` and `mobench_version` on manual Mobile Bench workflow
  branch-pinned validation.

## v0.1.32 and earlier

Status: historical test builds unless explicitly noted in package metadata. Do
not use for new integrations.

Earlier releases covered the initial CI contract, BrowserStack orchestration,
setup/teardown macro support, generated mobile templates, device matrices, and
consolidation of old `mobench-runner` functionality into `mobench-sdk`.

## Published Version History

| Version | Published | Published crates | Status |
| --- | --- | --- | --- |
| `v0.2.0` | 2026-07-31 | All nine workspace crates at `0.2.0` | Current supported release |
| `v0.1.49` | 2026-07-30 | `mobench 0.1.49`, `mobench-sdk 0.1.49`, `mobench-macros 0.1.49` | Superseded by `v0.2.0`; compatibility baseline |
| `v0.1.43` | 2026-07-05 | `mobench 0.1.43`, `mobench-sdk 0.1.43`, `mobench-macros 0.1.43` | Superseded by `v0.1.49` |
| `v0.1.42` | 2026-06-29 | `mobench 0.1.42`, `mobench-sdk 0.1.42`, `mobench-macros 0.1.42` | Superseded by `v0.1.43` |
| `v0.1.41` | 2026-05-14 | `mobench 0.1.41`, `mobench-sdk 0.1.41`, `mobench-macros 0.1.41` | Superseded by `v0.1.42` |
| `v0.1.37` | 2026-04-27 | `mobench 0.1.37`, `mobench-sdk 0.1.37`, `mobench-macros 0.1.37` | Superseded by `v0.1.41` |
| `v0.1.36` | 2026-04-27 | `mobench 0.1.36`, `mobench-sdk 0.1.36`, `mobench-macros 0.1.36` | Superseded by `v0.1.37` |
| `v0.1.35` | 2026-04-24 | `mobench 0.1.35`, `mobench-sdk 0.1.35`, `mobench-macros 0.1.35` | Superseded by `v0.1.36` |
| `v0.1.34` | 2026-04-23 | `mobench 0.1.34`, `mobench-sdk 0.1.34`, `mobench-macros 0.1.34` | Superseded by `v0.1.35` |
| `v0.1.33` | 2026-04-17 | `mobench 0.1.33`, `mobench-sdk 0.1.33`, `mobench-macros 0.1.33` | Superseded by `v0.1.34` |
| `v0.1.32` and earlier | 2026-01 through 2026-04 | `mobench`, `mobench-sdk`, `mobench-macros` pre-support publishes | Historical |
