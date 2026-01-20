# mobench

Mobile benchmarking SDK for Rust. Build and run Rust benchmarks on Android and iOS, locally or on BrowserStack, with a library-first workflow.

## What it is

mobench provides a Rust API and a CLI for running benchmarks on real mobile devices. You define benchmarks in Rust, generate mobile bindings automatically, and drive execution from the CLI with consistent output formats (JSON, Markdown, CSV).

## How mobench works

- `#[benchmark]` marks functions and registers them via `inventory`
- `mobench-sdk` builds mobile artifacts, provides the timing harness, and generates app templates from embedded assets
- UniFFI proc macros generate Kotlin and Swift bindings directly from Rust types
- The CLI writes a benchmark spec (function, iterations, warmup) and packages it into the app
- Mobile apps call `run_benchmark` via the generated bindings and return timing samples
- The CLI collects results locally or from BrowserStack and writes summaries

## Workspace crates

- `crates/mobench` ([mobench](https://crates.io/crates/mobench)): CLI tool that builds, runs, and fetches benchmarks
- `crates/mobench-sdk` ([mobench-sdk](https://crates.io/crates/mobench-sdk)): core SDK with timing harness, builders, registry, and codegen
- `crates/mobench-macros` ([mobench-macros](https://crates.io/crates/mobench-macros)): `#[benchmark]` proc macro
- `crates/sample-fns`: sample benchmarks and UniFFI bindings
- `examples/basic-benchmark`: minimal SDK integration example
- `examples/ffi-benchmark`: full UniFFI/FFI surface example

## Quick start

```bash
# Install the CLI
cargo install mobench

# Add the SDK to your project
cargo add mobench-sdk inventory

# Build artifacts (outputs to target/mobench/ by default)
cargo mobench build --target android
cargo mobench build --target ios

# Run a benchmark locally
cargo mobench run --target android --function sample_fns::fibonacci

# Run on BrowserStack (use --release for smaller APK uploads)
cargo mobench run --target android --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" --release
```

## Configuration

mobench supports a `mobench.toml` configuration file for project settings:

```toml
[project]
crate = "bench-mobile"
library_name = "bench_mobile"

[android]
package = "com.example.bench"
min_sdk = 24

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_crate::my_benchmark"
default_iterations = 100
default_warmup = 10
```

CLI flags override config file values when provided.

## Project docs

- `BENCH_SDK_INTEGRATION.md`: SDK integration guide
- `BUILD.md`: build prerequisites and troubleshooting
- `TESTING.md`: testing guide and device workflows
- `BROWSERSTACK_CI_INTEGRATION.md`: BrowserStack CI setup
- `FETCH_RESULTS_GUIDE.md`: fetching and summarizing results
- `PROJECT_PLAN.md`: goals and backlog
- `CLAUDE.md`: developer guide

## Release Notes

### v0.1.12

- **Fix iOS XCUITest BrowserStack detection**: Added Info.plist to the UITests target template, resolving issues where BrowserStack could not properly detect and run XCUITest bundles
- **Improved video capture for BrowserStack**: Increased post-benchmark delay from 0.5s to 5.0s to ensure benchmark results are captured in BrowserStack video recordings
- **Better UX during benchmark runs**: iOS app now shows "Running benchmarks..." text before results appear, providing visual feedback during execution
- **Template sync**: Synchronized top-level iOS/Android templates with SDK-embedded templates for consistency

### v0.1.11

- Initial public release with `--release` flag support
- `package-xcuitest` command for iOS BrowserStack testing
- Updated mobile timing display and documentation

MIT licensed — World Foundation 2026.
