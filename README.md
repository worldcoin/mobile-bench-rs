# mobench

Mobile benchmarking SDK for Rust. Build and run Rust benchmarks on Android and iOS, locally or on BrowserStack, with a library-first workflow.

## What it is

mobench provides a Rust API and a CLI for running benchmarks on real mobile devices. You define benchmarks in Rust, generate mobile bindings automatically, and drive execution from the CLI with consistent output formats (JSON, Markdown, CSV).

## How mobench works

- `#[benchmark]` marks functions and registers them via `inventory`
- `mobench-sdk` builds mobile artifacts and generates app templates from embedded assets
- UniFFI proc macros generate Kotlin and Swift bindings directly from Rust types
- The CLI writes a benchmark spec (function, iterations, warmup) and packages it into the app
- Mobile apps call `run_benchmark` via the generated bindings and return timing samples
- The CLI collects results locally or from BrowserStack and writes summaries

## Workspace crates

- `crates/mobench` ([mobench](https://crates.io/crates/mobench)): CLI tool that builds, runs, and fetches benchmarks
- `crates/mobench-sdk` ([mobench-sdk](https://crates.io/crates/mobench-sdk)): core SDK (builders, registry, codegen)
- `crates/mobench-macros` ([mobench-macros](https://crates.io/crates/mobench-macros)): `#[benchmark]` proc macro
- `crates/mobench-runner` ([mobench-runner](https://crates.io/crates/mobench-runner)): lightweight timing harness
- `crates/bench-cli`: BrowserStack and CLI support utilities
- `crates/bench-runner`: host-side harness utilities
- `crates/sample-fns`: sample benchmarks and UniFFI bindings
- `examples/basic-benchmark`: example SDK integration crate

## Quick start

```bash
# Install the CLI
cargo install mobench

# Add the SDK to your project
cargo add mobench-sdk inventory

# Build artifacts
cargo mobench build --target android
cargo mobench build --target ios

# Run a benchmark
cargo mobench run --target android --function sample_fns::fibonacci
```

## Project docs

- `BENCH_SDK_INTEGRATION.md`: SDK integration guide
- `BUILD.md`: build prerequisites and troubleshooting
- `TESTING.md`: testing guide and device workflows
- `BROWSERSTACK_CI_INTEGRATION.md`: BrowserStack CI setup
- `FETCH_RESULTS_GUIDE.md`: fetching and summarizing results
- `PROJECT_PLAN.md`: goals and backlog
- `CLAUDE.md`: developer guide

MIT licensed — World Foundation 2026.
