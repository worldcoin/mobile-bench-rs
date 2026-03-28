# Architecture

Updated: 2026-03-27

## System shape

`mobench` now has two distinct but related product surfaces:

1. benchmark execution
   - build mobile artifacts
   - run locally or on BrowserStack
   - collect summaries, CSV, plots, and PR comments
2. native profiling
   - plan and execute local native captures
   - keep a normalized manifest contract
   - write flamegraph and semantic-phase artifacts

The workspace still centers on three published crates:

- `mobench`: CLI orchestration, BrowserStack client, CI/reporting entry points, profile execution, and flamegraph viewer generation
- `mobench-sdk`: timing harness, registry/runner, project generation, builders, and generated mobile runner templates
- `mobench-macros`: `#[benchmark]` proc macro registration

## Runtime layers

### CLI orchestration

Location: `crates/mobench/src/`

Responsibilities:
- parse commands and resolve project layout
- build/package Android and iOS artifacts
- dispatch benchmark runs locally or to BrowserStack
- fetch BrowserStack artifacts and enrich results
- manage fixture CI flows and report rendering
- run local native profiling and summarize profile sessions

Important modules:
- `lib.rs`: command parsing and benchmark/CI orchestration
- `profile.rs`: target resolution, capture planning, capture execution, manifest/summary writing
- `flamegraph_viewer.rs`: focused/full folded-stack derivation and interactive flamegraph HTML generation
- `browserstack.rs`: App Automate upload, schedule, polling, and fetch helpers

### SDK/runtime layer

Location: `crates/mobench-sdk/src/`

Responsibilities:
- benchmark timing and statistics
- function registry and runtime dispatch
- semantic phase capture via `profile_phase(...)`
- Android/iOS code generation and builders
- portable FFI-facing benchmark types

Important modules:
- `timing.rs`: `BenchSpec`, `BenchReport`, samples, phases, and the timing harness
- `registry.rs` / `runner.rs`: benchmark lookup and execution
- `builders/android.rs`, `builders/ios.rs`: native library build + project packaging
- `codegen.rs`: template expansion and regeneration rules

### Generated mobile runners

Generated from `crates/mobench-sdk/templates/` into `target/mobench/`.

Responsibilities:
- load `bench_spec.json` or launch-time overrides
- call the UniFFI-exposed Rust benchmark entrypoints
- emit benchmark JSON for the CLI fetch/parsing paths
- keep benchmark work alive long enough for local native profile capture when profiling launch options are supplied

Platform-specific behavior:
- Android runner emits `BENCH_JSON ...` in logcat and is marked `profileable` for local native capture
- iOS runner emits `BENCH_REPORT_JSON_START/END` markers and supports repeat/warmup launch options used by local simulator-host profiling

### CI/reporting layer

Responsibilities:
- run fixture benchmarks on BrowserStack from GitHub Actions
- publish normalized CI outputs (`summary.json`, `summary.md`, `results.csv`)
- render shared device-comparison plots
- publish sticky PR comments and check runs

Current status:
- benchmark CI is first-class
- local native profiling is first-class on developer machines
- a dedicated profiling self-test workflow is separate from BrowserStack benchmark workflows

## Key flows

### Benchmark flow

1. benchmark functions are registered at compile time through `inventory`
2. `cargo mobench build` or `run` resolves the benchmark crate/layout
3. the SDK builders compile Rust libraries and regenerate mobile bindings/templates
4. mobile runners execute `run_benchmark(...)`
5. the CLI collects and normalizes benchmark outputs locally or from BrowserStack
6. reporters render JSON, Markdown, CSV, plots, PR comments, and check-run summaries

### Profiling flow

1. `cargo mobench profile run` resolves a target and device context
2. `profile.rs` builds a deterministic manifest and output contract
3. supported local backends attempt native capture
   - Android: `simpleperf`
   - iOS: simulator-host `sample`
4. post-processing writes:
   - `stacks.folded`
   - `native-report.txt`
   - `flamegraph.full.svg`
   - `flamegraph.focused.svg`
   - `flamegraph.html`
   - `artifacts/semantic/phases.json` when phase data exists
5. `profile summarize` renders the manifest into Markdown or JSON

### Device resolution flow

The repo uses one device-resolution model for both benchmark CI and profile planning:

- `cargo mobench devices resolve` is the canonical entry point
- `profile run` reuses the same profile/matrix/device concepts
- BrowserStack benchmark workflows rely on this resolution path in CI

## Boundaries and non-goals

- BrowserStack benchmark execution is supported for timing/memory runs.
- BrowserStack native profiling is explicitly unsupported until retrievable native artifacts exist.
- The flamegraph viewer is a stable dual-view explorer; rolled-back experimental tower-collapse behavior is documented only as historical design work.
