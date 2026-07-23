# Architecture

Updated: 2026-07-23. Current release: `0.1.48`.

mobench has two product surfaces:

1. Benchmark execution: build mobile artifacts, run host-only, locally, or on
   BrowserStack, then write JSON, Markdown, CSV, plot, PR-comment, and
   check-run outputs.
2. Local native profiling: run local native capture plans, normalize profile
   manifests, and write flamegraph plus semantic-phase artifacts.

## Published Crates

- `mobench`: CLI orchestration, BrowserStack client, CI/reporting entry points,
  profile execution, flamegraph viewer generation, and programmatic CLI helpers.
- `mobench-sdk`: timing harness, benchmark registry, runner, generated runner
  backends, project generation, Android/iOS builders, UniFFI compatibility, and
  native C ABI exports.
- `mobench-macros`: `#[benchmark]` proc macro registration with setup,
  teardown, per-iteration setup, and compile-time signature validation.

## Runtime Layers

### CLI Orchestration

Location: `crates/mobench/src/`

Responsibilities:

- Parse command-line arguments and resolve project layout.
- Load and validate config, device matrix, and BrowserStack credentials.
- Build/package Android and iOS artifacts.
- Select generated runner backend from `[project].ffi_backend`.
- Dispatch benchmark runs host-only, locally, or on BrowserStack.
- Fetch BrowserStack artifacts and normalize run outputs.
- Render JSON, Markdown, CSV, plots, PR comments, and check-run summaries.
- Run local native profiling, summarize profiles, and generate profile diffs.

Important modules:

- `cli.rs`: clap command surface and value enums.
- `lib.rs`: command dispatch, benchmark/CI orchestration, report helpers, and
  programmatic API types.
- `config.rs`: config loading, validation, and `ffi_backend` resolution.
- `doctor.rs`: prerequisite checks and validation issue rendering.
- `browserstack.rs`: App Automate upload, scheduling, polling, and artifact
  fetching.
- `github.rs`: GitHub comment/check integration helpers.
- `plots.rs`: CI plot input extraction and rendering.
- `profile.rs`: profiling backend planning/execution, manifests, summaries,
  and diffs.
- `flamegraph_viewer.rs`: folded-stack derivation, SVG generation, and
  interactive viewer assembly.

### SDK Runtime And Builders

Location: `crates/mobench-sdk/src/`

Responsibilities:

- Time warmup and measured iterations.
- Register and discover benchmark functions through `inventory`.
- Execute benchmarks through `run_benchmark`.
- Generate Android and iOS runner projects.
- Build native libraries and package mobile artifacts.
- Provide UniFFI-compatible types.
- Export the native JSON C ABI through `export_native_c_abi!()`.
- Record semantic profiling phases through `profile_phase`.

Generated runner backends:

- `uniffi`: default compatibility backend using generated Kotlin/Swift bindings.
- `native-c-abi`: direct mobench JSON C ABI backend. Benchmark crates export
  `mobench_run_benchmark_json`, `mobench_free_buf`, and
  `mobench_last_error_message` through `mobench_sdk::export_native_c_abi!()`.

### Generated Mobile Runners

Generated Android and iOS runners:

- Read `bench_spec.json`.
- Apply runtime overrides from Android intent extras or iOS environment
  variables/launch arguments.
- Execute the requested benchmark.
- Emit benchmark JSON markers that the CLI can parse from local or
  BrowserStack logs.
- Keep profiling launch options available for local native capture.

Platform-specific behavior:

- Android emits `BENCH_JSON ...` in logcat and can be marked `profileable` for
  local native capture.
- iOS emits `BENCH_REPORT_JSON_START/END` markers and supports simulator-host
  profiling launch options.

### CI And Reporting

Responsibilities:

- Run benchmark fixtures from GitHub Actions.
- Publish normalized CI outputs: `summary.json`, `summary.md`, `results.csv`,
  and optional `plots/*.svg`.
- Compare against baselines and write optional JUnit output.
- Publish sticky PR comments and GitHub Check Run summaries.

Fork-PR BrowserStack CI has separate privilege domains:

- `ci prepare` runs build-time caller code without secrets or write permission
  and emits only enumerated mobile packages plus a cryptographic manifest.
- `ci run-prebuilt` runs from a trusted immutable mobench release, verifies the
  handoff, and performs provider operations without checking out or executing
  the caller on the credentialed runner.
- Platform-specific function/device selections are normalized as data. Each
  function is packaged once, and trusted result handling rejects incomplete or
  duplicate function/device shards before writing canonical summaries.
- summarization is read-only, while PR/check publishing is isolated in a job
  with only its narrow write permission.

This split is an execution boundary, not merely a workflow organization
convention. See
[Reusable Workflow Security](../guides/reusable-workflow-security.md).

BrowserStack benchmark timing/resource metrics are supported. BrowserStack
native stack/flamegraph profiling is explicitly unsupported in this release.

## Key Flows

### Benchmark Flow

1. Benchmark functions register at compile time through `inventory`.
2. `cargo mobench build`, `run`, or `ci run` resolves the project and benchmark
   crate from flags, config, Cargo metadata, git root, or legacy fallback.
3. SDK builders compile native libraries and generate backend-specific runners.
4. Mobile runners execute the benchmark through UniFFI or native C ABI.
5. CLI collects and normalizes local or BrowserStack outputs.
6. Reporters render JSON, Markdown, CSV, plots, PR comments, and check-run
   summaries.

### Profiling Flow

1. `cargo mobench profile run` resolves target, backend, provider, and device
   context.
2. `profile.rs` writes a deterministic profile manifest output contract.
3. Supported local backends attempt native capture:
   - Android: `simpleperf`.
   - iOS: simulator-host `sample`.
   - Rust tracing: planned manifest/trace contract.
4. Post-processing writes folded stacks, native reports, flamegraph SVGs,
   `flamegraph.html`, and semantic phase data when available.
5. `profile summarize` renders Markdown or JSON.
6. `profile diff` compares two profile manifests and writes a diff bundle.

### Device Resolution Flow

- `cargo mobench devices resolve` is the canonical deterministic resolver.
- `ci run` can resolve devices from matrix files and tags.
- `profile run` reuses the same device, OS version, profile, and matrix concepts
  for planning.

## Boundaries

- The SDK owns benchmark authoring, timing, registry, generated runners, and
  mobile builders.
- The CLI owns orchestration, provider access, output contracts, reporting, and
  profiling commands.
- Generated templates are implementation artifacts, but their input/output
  paths are compatibility-sensitive because downstream projects may depend on
  them.
