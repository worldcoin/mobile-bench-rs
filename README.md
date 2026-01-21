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

# Check prerequisites before building
cargo mobench check --target android
cargo mobench check --target ios

# Build artifacts (outputs to target/mobench/ by default)
cargo mobench build --target android
cargo mobench build --target ios

# Build with progress output for clearer feedback
cargo mobench build --target android --progress

# Run a benchmark locally
cargo mobench run --target android --function sample_fns::fibonacci

# Run on BrowserStack (use --release for smaller APK uploads)
cargo mobench run --target android --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" --release

# List available BrowserStack devices
cargo mobench devices --platform android

# View benchmark results summary
cargo mobench summary results.json
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

## Setup and Teardown

For benchmarks that require expensive setup (like generating test data or initializing connections), you can exclude setup time from measurements using the `setup` attribute.

### The Problem

Without setup/teardown, expensive initialization is measured as part of your benchmark:

```rust
#[benchmark]
fn verify_proof() {
    let proof = generate_complex_proof();  // This is measured (bad!)
    verify(&proof);                         // This is what we want to measure
}
```

### The Solution

Use the `setup` attribute to run initialization once before timing begins:

```rust
// Setup function runs once before all iterations (not timed)
fn setup_proof() -> ProofInput {
    generate_complex_proof()  // Takes 5 seconds, but not measured
}

#[benchmark(setup = setup_proof)]
fn verify_proof(input: &ProofInput) {
    verify(&input.proof);  // Only this is measured
}
```

### Per-Iteration Setup

For benchmarks that mutate their input, use `per_iteration` to get fresh data each iteration:

```rust
fn generate_random_vec() -> Vec<i32> {
    (0..1000).map(|_| rand::random()).collect()
}

#[benchmark(setup = generate_random_vec, per_iteration)]
fn sort_benchmark(data: Vec<i32>) {
    let mut data = data;
    data.sort();  // Each iteration gets a fresh unsorted vec
}
```

### Setup with Teardown

For resources that need cleanup (database connections, temp files, etc.):

```rust
fn setup_db() -> Database { Database::connect("test.db") }
fn cleanup_db(db: Database) { db.close(); std::fs::remove_file("test.db").ok(); }

#[benchmark(setup = setup_db, teardown = cleanup_db)]
fn db_query(db: &Database) {
    db.query("SELECT * FROM users");
}
```

### When to Use Each Pattern

| Pattern | Use Case |
|---------|----------|
| `#[benchmark]` | Simple benchmarks with no setup or fast inline setup |
| `#[benchmark(setup = fn)]` | Expensive one-time setup, reused across iterations |
| `#[benchmark(setup = fn, per_iteration)]` | Benchmarks that mutate input, need fresh data each time |
| `#[benchmark(setup = fn, teardown = fn)]` | Resources requiring cleanup (connections, files, etc.) |

## Release Notes

### v0.1.15

- **Setup and teardown support**: `#[benchmark]` macro now supports `setup`, `teardown`, and `per_iteration` attributes for excluding expensive initialization from timing measurements
  ```rust
  fn setup_data() -> Vec<u8> { vec![0u8; 10_000_000] }

  #[benchmark(setup = setup_data)]
  fn process_data(data: &Vec<u8>) {
      // Only this is measured, not the setup
  }
  ```

### v0.1.14

- **New `check` command**: Validates prerequisites (NDK, Xcode, Rust targets, etc.) before building
  ```bash
  cargo mobench check --target android
  cargo mobench check --target ios
  ```
- **New `verify` command**: Validates registry, spec, and artifacts
- **New `summary` command**: Displays benchmark result statistics (avg/min/max/median)
- **New `devices` command**: Lists available BrowserStack devices with validation
- **`--progress` flag**: Simplified step-by-step output for `build` and `run` commands
- **SDK improvements**:
  - `#[benchmark]` macro now validates function signature at compile time (no params, returns `()`)
  - New `debug_benchmarks!()` macro for verifying benchmark registration
  - Better error messages with available benchmarks list
- **BrowserStack improvements**:
  - Better credential error messages with setup instructions
  - Artifact pre-flight validation before uploads
  - Upload progress indication with file sizes
  - Dashboard link printed immediately when build starts
  - Improved device fuzzy matching with suggestions

### v0.1.13

- **Fix iOS XCUITest test name mismatch**: Changed BrowserStack `only-testing` filter to use `testLaunchAndCaptureBenchmarkReport` which matches what BrowserStack parses from the xctest bundle

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
