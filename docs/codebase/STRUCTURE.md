# Structure

Updated: 2026-06-29. Release line: `0.1.42`.

## Workspace Layout

```text
mobile-bench-rs/
├── crates/
│   ├── mobench/          # CLI, BrowserStack client, reports, profiling
│   ├── mobench-sdk/      # timing, registry, builders, codegen, templates
│   ├── mobench-macros/   # #[benchmark] proc macro
│   └── sample-fns/       # repository demo benchmark crate
├── examples/
│   ├── basic-benchmark/  # minimal SDK integration example
│   └── ffi-benchmark/    # full generated FFI example
├── android/              # checked-in Android runner/demo app
├── ios/                  # checked-in iOS runner/demo app
├── templates/            # editable template sources
├── docs/
│   ├── guides/           # user-facing guides
│   ├── codebase/         # this reference set
│   ├── specs/            # current product/API spec
│   ├── diagrams/         # Mermaid source diagrams
│   └── schemas/          # machine-readable contracts
├── .github/
│   ├── actions/mobench/  # composite action wrapper
│   └── workflows/        # CI, benchmark, PR, and self-test workflows
```

## Important Code Locations

### CLI

- `crates/mobench/src/main.rs`: `mobench` binary entry point.
- `crates/mobench/src/bin/cargo-mobench.rs`: Cargo subcommand wrapper.
- `crates/mobench/src/cli.rs`: clap command surface.
- `crates/mobench/src/lib.rs`: command dispatch, benchmark orchestration,
  reporting helpers, and programmatic API types.
- `crates/mobench/src/config.rs`: config and matrix loading.
- `crates/mobench/src/doctor.rs`: prerequisite and config validation.
- `crates/mobench/src/browserstack.rs`: BrowserStack REST client.
- `crates/mobench/src/github.rs`: GitHub report/check helpers.
- `crates/mobench/src/plots.rs`: CI plot rendering helpers.
- `crates/mobench/src/profile.rs`: local native profiling and profile diffs.
- `crates/mobench/src/flamegraph_viewer.rs`: flamegraph SVG/viewer generation.
- `crates/mobench/src/summarize.rs`: benchmark summary parsing/rendering.

### SDK

- `crates/mobench-sdk/src/lib.rs`: public SDK facade and re-exports.
- `crates/mobench-sdk/src/timing.rs`: `BenchSpec`, samples, reports,
  statistics, semantic phases, and timing harness.
- `crates/mobench-sdk/src/types.rs`: shared target/build/config types.
- `crates/mobench-sdk/src/registry.rs`: inventory-backed discovery.
- `crates/mobench-sdk/src/runner.rs`: benchmark execution.
- `crates/mobench-sdk/src/native_c_abi.rs`: JSON C ABI exports.
- `crates/mobench-sdk/src/ffi.rs`: FFI-facing report types.
- `crates/mobench-sdk/src/uniffi_types.rs`: UniFFI-compatible types.
- `crates/mobench-sdk/src/codegen.rs`: template expansion and project
  generation.
- `crates/mobench-sdk/src/builders/android.rs`: Android build automation.
- `crates/mobench-sdk/src/builders/ios.rs`: iOS build/package automation.

### Macros

- `crates/mobench-macros/src/lib.rs`: `#[benchmark]` implementation and
  compile-time validation.

### Templates

- `templates/android/` and `templates/ios/`: editable template sources.
- `crates/mobench-sdk/templates/`: embedded templates compiled into the SDK.

Keep editable templates and embedded SDK templates in sync when template
behavior changes.

### Workflows

- `.github/actions/mobench/action.yml`: composite action wrapper.
- `.github/workflows/rust.yml`: Rust quality workflow.
- `.github/workflows/compile-gate.yml`: compile gate.
- `.github/workflows/mobile-bench.yml`: dispatchable benchmark workflow.
- `.github/workflows/reusable-bench.yml`: reusable benchmark workflow.
- `.github/workflows/mobile-bench-pr-auto.yml`: automatic PR benchmark dispatch.
- `.github/workflows/mobile-bench-pr-command.yml`: command-triggered PR
  benchmark dispatch.
- `.github/workflows/mobile-bench-profile-selftest.yml`: local profiling
  artifact self-test.
- `.github/workflows/mobile-bench-plot-fixtures.yml`: plot fixture validation.
- `.github/workflows/mobile-bench-selftest.yml`: benchmark self-test workflow.
- `.github/workflows/mobile-bench-action-example.yml`: action usage example.

## Generated Outputs

Default build output root:

```text
target/mobench/
```

Benchmark CI contract:

```text
target/mobench/ci/
├── summary.json
├── summary.md
├── results.csv
└── plots/
```

BrowserStack fetch output:

```text
target/browserstack/
```

Profile output:

```text
target/mobench/profile/
├── profile.json
├── summary.md
├── <run-id>/
└── artifacts/
    ├── raw/
    ├── processed/
    └── semantic/
```

## Where To Add New Work

- New CLI arguments: `crates/mobench/src/cli.rs`.
- New CLI orchestration/reporting behavior: `crates/mobench/src/lib.rs` or a
  focused module under `crates/mobench/src/`.
- New prerequisite/config validation: `crates/mobench/src/doctor.rs` or
  `crates/mobench/src/config.rs`.
- New profiling behavior: `crates/mobench/src/profile.rs`.
- New SDK runtime/build/codegen behavior: `crates/mobench-sdk/src/`.
- Benchmark registration semantics: `crates/mobench-macros/src/lib.rs`.
- Template/runtime UX changes: `templates/`, mirrored into
  `crates/mobench-sdk/templates/`.
- CI workflows and PR automation: `.github/workflows/`.
- Fixture smoke helpers: `examples/` and `crates/sample-fns/`.
