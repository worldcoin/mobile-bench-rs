# Structure

Updated: 2026-03-27

## Workspace layout

```text
mobile-bench-rs/
├── crates/
│   ├── mobench/                  # CLI, BrowserStack client, profile execution, reports
│   ├── mobench-sdk/              # timing/runtime/builders/codegen/templates
│   ├── mobench-macros/           # #[benchmark] proc macro
│   └── sample-fns/               # sample UniFFI benchmark crate used for smoke tests
├── examples/
│   ├── basic-benchmark/          # minimal benchmark crate
│   └── ffi-benchmark/            # fixture benchmark crate used in CI
├── android/                      # checked-in Android runner/demo app
├── ios/                          # checked-in iOS runner/demo app
├── templates/                    # editable source templates mirrored into SDK templates
├── docs/
│   ├── adr/
│   ├── codebase/                 # this reference set
│   ├── plans/                    # design and implementation notes
│   └── schemas/
├── .github/
│   ├── actions/mobench/          # local composite action for benchmark CI
│   └── workflows/                # benchmark, fixture, plot, and PR-dispatch workflows
└── scripts/                      # fixture verification and helper scripts
```

## Important code locations

### CLI

- `crates/mobench/src/lib.rs`: clap surface and benchmark/CI command orchestration
- `crates/mobench/src/profile.rs`: local native profiling flow, manifests, summaries, artifact contracts
- `crates/mobench/src/flamegraph_viewer.rs`: focused/full flamegraph generation and interactive HTML viewer
- `crates/mobench/src/browserstack.rs`: BrowserStack App Automate REST client
- `crates/mobench/src/config.rs`: config + matrix loading

### SDK

- `crates/mobench-sdk/src/timing.rs`: `BenchSpec`, `BenchReport`, sample timing, semantic phases
- `crates/mobench-sdk/src/registry.rs`: benchmark discovery
- `crates/mobench-sdk/src/runner.rs`: `run_benchmark` entrypoints
- `crates/mobench-sdk/src/codegen.rs`: template expansion and regeneration
- `crates/mobench-sdk/src/builders/android.rs`: Android library build, packaging, and template sync
- `crates/mobench-sdk/src/builders/ios.rs`: iOS library build, Xcode project generation, IPA/XCUITest packaging

### Generated templates

- `crates/mobench-sdk/templates/android/`: embedded Android runner template
- `crates/mobench-sdk/templates/ios/BenchRunner/`: embedded iOS runner template
- `templates/android/` and `templates/ios/`: editable template sources mirrored into the SDK tree

### Repository fixtures and demos

- `android/`: checked-in Android app used for local smoke work and template parity
- `ios/BenchRunner/`: checked-in iOS app used for local smoke work and template parity
- `crates/sample-fns/`: sample benchmarks used for flamegraph and device-resolution smoke tests
- `examples/ffi-benchmark/`: benchmark fixture exercised by GitHub Actions

## Generated output layout

Default output root: `target/mobench/`

Common subtrees:
- `android/`: generated Android project and APK/test APK outputs
- `ios/`: generated iOS project, app, IPA, XCUITest bundle, and framework outputs
- `ci/`: standardized benchmark workflow outputs (`summary.json`, `summary.md`, `results.csv`, plots)
- `profile/`: run-scoped local profiling sessions and latest-run convenience copies

Profile session layout:

```text
target/mobench/profile/<run-id>/
├── profile.json
├── summary.md
└── artifacts/
    ├── raw/
    ├── processed/
    └── semantic/
```

## Where to add new work

- new CLI/report/profile behavior: `crates/mobench/src/`
- new SDK/runtime/build/codegen behavior: `crates/mobench-sdk/src/`
- benchmark registration semantics: `crates/mobench-macros/src/lib.rs`
- template/runtime UX changes: `templates/` first, then mirror into `crates/mobench-sdk/templates/`
- CI workflows or PR automation: `.github/workflows/`
- benchmark fixtures or local smoke helpers: `examples/`, `crates/sample-fns/`, `scripts/ci/`
