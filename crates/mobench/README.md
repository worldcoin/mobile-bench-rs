# mobench

Mobile benchmarking CLI for Rust - Run benchmarks on real Android and iOS devices.

The `mobench` CLI is the easiest way to benchmark your Rust code on mobile devices. It handles everything from project setup to building mobile apps to running tests on real devices via BrowserStack.

## Installation

```bash
cargo install mobench
```

Or use as a Cargo subcommand:

```bash
cargo install mobench
cargo mobench --help
```

## Quick Start

### 1. Initialize Your Project

```bash
# Create mobile benchmarking setup for Android
cargo mobench init --target android

# Or for iOS
cargo mobench init --target ios

# Or for both platforms
cargo mobench init --target both
```

This creates:
- `bench-mobile/` - FFI wrapper crate with UniFFI bindings
- `android/` or `ios/` - Platform-specific app projects (generated to output directory)
- `bench-config.toml` - Run configuration file
- `mobench.toml` - Project configuration file (when using `init`)
- `benches/example.rs` - Example benchmarks (with `--examples`)

### 2. Write Benchmarks

```rust
// benches/my_benchmarks.rs
use mobench_sdk::benchmark;

#[benchmark]
fn fibonacci_30() {
    let result = fibonacci(30);
    std::hint::black_box(result);
}

fn fibonacci(n: u32) -> u64 {
    match n {
        0 => 0,
        1 => 1,
        _ => fibonacci(n - 1) + fibonacci(n - 2),
    }
}
```

### 3. Build for Mobile

```bash
# Build Android APK
cargo mobench build --target android

# Build iOS app
cargo mobench build --target ios
```

### 4. Run Benchmarks

Local device workflow (builds artifacts and writes the run spec; launch the app manually):
```bash
cargo mobench run --target android --function fibonacci_30 --iterations 50
```

On real devices via BrowserStack:
```bash
export BROWSERSTACK_USERNAME=your_username
export BROWSERSTACK_ACCESS_KEY=your_access_key

cargo mobench run \
  --target android \
  --function fibonacci_30 \
  --devices "Google Pixel 7-13.0" \
  --iterations 100 \
  --warmup 10 \
  --release
```

**Note**: Always use the `--release` flag for BrowserStack runs. Debug builds are significantly larger (~544MB vs ~133MB for release) and may cause upload timeouts.

## Commands

### `init` - Initialize Project

Create mobile benchmarking infrastructure:

```bash
cargo mobench init [OPTIONS]
```

**Options:**
- `--target <android|ios|both>` - Target platform (default: android)
- `--output <FILE>` - Config file path (default: bench-config.toml)

**Example:**
```bash
cargo mobench init --target both --output my-bench.toml
```

### `build` - Build Mobile Apps

Cross-compile and package for mobile platforms:

```bash
cargo mobench build --target <android|ios> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform to build for (required)
- `--release` - Build in release mode (default: debug)
- `--output-dir <DIR>` - Output directory for mobile artifacts (default: `target/mobench/`)
- `--crate-path <PATH>` - Path to the benchmark crate (default: auto-detect)
- `--dry-run` - Print what would be done without making changes
- `--verbose` / `-v` - Print verbose output including all commands

**Examples:**
```bash
# Build Android APK in release mode
cargo mobench build --target android --release

# Build iOS xcframework
cargo mobench build --target ios

# Preview build without making changes
cargo mobench build --target android --dry-run

# Build with verbose output
cargo mobench build --target ios --verbose

# Build to custom output directory
cargo mobench build --target android --output-dir ./my-output
```

**Outputs:**
- Android: `target/mobench/android/app/build/outputs/apk/debug/app-debug.apk`
- iOS: `target/mobench/ios/sample_fns.xcframework`

### `run` - Run Benchmarks

Execute benchmarks on devices:

```bash
cargo mobench run --target <android|ios> --function <NAME> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform (required)
- `--function <NAME>` - Benchmark function name (required)
- `--iterations <N>` - Number of iterations (default: 100)
- `--warmup <N>` - Warmup iterations (default: 10)
- `--devices <LIST>` - Comma-separated device list for BrowserStack
- `--local-only` - Skip mobile builds (no device run)
- `--config <FILE>` - Load run spec from config file
- `--ios-app <FILE>` - iOS .ipa or zipped .app for BrowserStack
- `--ios-test-suite <FILE>` - iOS XCUITest runner (.zip or .ipa)
- `--output <FILE>` - Save results to JSON file (default: run-summary.json)
- `--summary-csv` - Write CSV summary alongside JSON/Markdown
- `--fetch` - Fetch BrowserStack results after completion

**Outputs:**
- JSON summary (default: `run-summary.json`)
- Markdown summary (same base name, `.md`)
- CSV summary (same base name, `.csv`, when `--summary-csv` is set)

**Examples:**
```bash
# Run locally (no BrowserStack devices specified)
cargo mobench run --target android --function fibonacci_30

# Run on BrowserStack devices (use --release for smaller APK)
cargo mobench run \
  --target android \
  --function sha256_hash \
  --devices "Google Pixel 7-13.0,Samsung Galaxy S23-13.0" \
  --iterations 50 \
  --release \
  --output results.json

# Run on iOS with auto-fetch (use --release for smaller artifacts)
cargo mobench run \
  --target ios \
  --function json_parse \
  --devices "iPhone 14-16,iPhone 15-17" \
  --release \
  --fetch
```

### `package-ipa` - Package iOS IPA

Create a signed IPA for BrowserStack:

```bash
cargo mobench package-ipa [OPTIONS]
```

**Options:**
- `--scheme <NAME>` - Xcode scheme (default: BenchRunner)
- `--method <adhoc|development>` - Signing method (default: adhoc)

**Example:**
```bash
cargo mobench package-ipa --method adhoc
```

**Output:** `target/mobench/ios/BenchRunner.ipa`

### `package-xcuitest` - Package XCUITest Runner

Create the XCUITest runner package required for BrowserStack iOS testing:

```bash
cargo mobench package-xcuitest [OPTIONS]
```

**Options:**
- `--scheme <NAME>` - Xcode scheme for UI tests (default: BenchRunnerUITests)

**Example:**
```bash
cargo mobench package-xcuitest
```

**Output:** `target/mobench/ios/BenchRunnerUITests.zip`

This command builds the XCUITest target and packages it into the zip format that BrowserStack expects for iOS test automation.

### `plan` - Generate Device Matrix

Create a template device matrix file:

```bash
cargo mobench plan [--output <FILE>]
```

**Example:**
```bash
cargo mobench plan --output devices.yaml
```

**Output:** `device-matrix.yaml`

```yaml
devices:
  - name: Google Pixel 7-13.0
    os: android
    os_version: "13.0"
    tags: [default, pixel]
  - name: iPhone 14-16
    os: ios
    os_version: "16"
    tags: [default, iphone]
```

### `list` - List Benchmarks

Show benchmarks discovered via `#[benchmark]`:

```bash
cargo mobench list
```

### `fetch` - Fetch Results

Download BrowserStack build artifacts:

```bash
cargo mobench fetch --target <android|ios> --build-id <ID> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform (required)
- `--build-id <ID>` - BrowserStack build ID (required)
- `--output-dir <DIR>` - Download directory (default: target/browserstack)

**Example:**
```bash
cargo mobench fetch \
  --target android \
  --build-id abc123def456 \
  --output-dir ./results
```

### `compare` - Compare Summaries

Compare two JSON run summaries and emit a Markdown report:

```bash
cargo mobench compare \
  --baseline results-v1.json \
  --candidate results-v2.json \
  --output comparison.md
```

## Configuration

### Project Configuration (`mobench.toml`)

mobench automatically loads `mobench.toml` from the current directory or any parent directory:

```toml
[project]
# Name of the benchmark crate
crate = "bench-mobile"

# Rust library name (typically crate name with hyphens replaced by underscores)
library_name = "bench_mobile"

# Output directory for build artifacts (default: target/mobench/)
# output_dir = "target/mobench"

[android]
# Android package name
package = "com.example.bench"

# Minimum Android SDK version (default: 24)
min_sdk = 24

# Target Android SDK version (default: 34)
target_sdk = 34

[ios]
# iOS bundle identifier
bundle_id = "com.example.bench"

# iOS deployment target version (default: 15.0)
deployment_target = "15.0"

# Development team ID for code signing (optional)
# team_id = "YOUR_TEAM_ID"

[benchmarks]
# Default benchmark function to run
default_function = "my_crate::my_benchmark"

# Default number of benchmark iterations
default_iterations = 100

# Default number of warmup iterations
default_warmup = 10
```

CLI flags always override config file values when provided.

### Run Config File Format (`bench-config.toml`)

For BrowserStack runs, you can also use a separate run configuration:

```toml
target = "android"
function = "sample_fns::fibonacci"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"
device_tags = ["default"] # optional; filter devices by tag

[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "my-project-benchmarks"

[ios_xcuitest]
app = "target/mobench/ios/BenchRunner.ipa"
test_suite = "target/mobench/ios/BenchRunnerUITests.zip"
```

### Device Matrix Format (`device-matrix.yaml`)

```yaml
devices:
  - name: "Google Pixel 7-13.0"
    os: "android"
    os_version: "13.0"
    tags: ["default", "pixel"]
  - name: "iPhone 14-16"
    os: "ios"
    os_version: "16"
    tags: ["default", "iphone"]
```

### Environment Variables

BrowserStack credentials can be provided via:

1. **Environment variables** (recommended):
   ```bash
   export BROWSERSTACK_USERNAME=your_username
   export BROWSERSTACK_ACCESS_KEY=your_access_key
   ```

2. **`.env.local` file**:
   ```
   BROWSERSTACK_USERNAME=your_username
   BROWSERSTACK_ACCESS_KEY=your_access_key
   ```

3. **Config file** with variable expansion:
   ```toml
   [browserstack]
   username = "${BROWSERSTACK_USERNAME}"
   access_key = "${BROWSERSTACK_ACCESS_KEY}"
   ```

## Requirements

### For Android

- **Android NDK** - Set `ANDROID_NDK_HOME` environment variable
- **cargo-ndk** - Install with `cargo install cargo-ndk`
- **Android SDK** - API level 24+ required
- **Gradle** - For building APKs (bundled with Android project)

### For iOS

- **macOS** with Xcode installed
- **Xcode Command Line Tools** - `xcode-select --install`
- **Rust iOS targets**:
  ```bash
  rustup target add aarch64-apple-ios
  rustup target add aarch64-apple-ios-sim
  ```
- **XcodeGen** - Install with `brew install xcodegen`

### For BrowserStack

- **BrowserStack App Automate account** - [Sign up](https://www.browserstack.com/app-automate)
- **Credentials** - Username and access key from account settings

## Examples

### Benchmark Crypto Operations

```bash
# Initialize
cargo mobench init --target android

# Add benchmark
cat > benches/crypto.rs <<'EOF'
use mobench_sdk::benchmark;
use sha2::{Sha256, Digest};

#[benchmark]
fn sha256_1kb() {
    let data = vec![0u8; 1024];
    let hash = Sha256::digest(&data);
    std::hint::black_box(hash);
}
EOF

# Build
cargo mobench build --target android --release

# Run on multiple devices (use --release for BrowserStack)
cargo mobench run \
  --target android \
  --function sha256_1kb \
  --devices "Google Pixel 7-13.0,Samsung Galaxy S23-13.0,OnePlus 11-13.0" \
  --iterations 200 \
  --release \
  --output crypto-results.json
```

### Compare iOS Performance

```bash
# Run same benchmark on different iOS versions (use --release for BrowserStack)
cargo mobench run \
  --target ios \
  --function json_parse \
  --devices "iPhone 13-15,iPhone 14-16,iPhone 15-17" \
  --iterations 100 \
  --release \
  --fetch \
  --output ios-comparison.json
```

### CI Integration

```yaml
# .github/workflows/mobile-bench.yml
name: Mobile Benchmarks

on: [push]

jobs:
  benchmark:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install mobench
        run: cargo install mobench

      - name: Setup Android NDK
        uses: nttld/setup-ndk@v1
        with:
          ndk-version: r25c

      - name: Build
        run: cargo mobench build --target android --release

      - name: Run benchmarks
        env:
          BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
          BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
        run: |
          cargo mobench run \
            --target android \
            --function my_benchmark \
            --devices "Google Pixel 7-13.0" \
            --iterations 50 \
            --release \
            --output results.json \
            --fetch

      - name: Upload results
        uses: actions/upload-artifact@v3
        with:
          name: benchmark-results
          path: results.json
```

## Workflow

```
┌─────────────────────┐
│ 1. cargo mobench    │
│    init             │
└──────────┬──────────┘
           │
           ↓
┌─────────────────────┐
│ 2. Write benchmarks │
│    with #[benchmark]│
└──────────┬──────────┘
           │
           ↓
┌─────────────────────┐
│ 3. cargo mobench    │
│    build            │
└──────────┬──────────┘
           │
           ↓
┌─────────────────────┐
│ 4. cargo mobench    │
│    run              │
└──────────┬──────────┘
           │
      ┌────┴────┐
      ↓         ↓
┌──────────┐ ┌──────────────┐
│  Local   │ │ BrowserStack │
│ Emulator │ │ Real Devices │
└──────────┘ └──────────────┘
```

## Troubleshooting

### Android NDK not found

```bash
export ANDROID_NDK_HOME=/path/to/ndk
```

Or install via Android Studio: Tools → SDK Manager → SDK Tools → NDK

### iOS code signing issues

For BrowserStack testing, use ad-hoc signing:

```bash
cargo mobench package-ipa --method adhoc
```

### BrowserStack authentication failed

Verify credentials:

```bash
echo $BROWSERSTACK_USERNAME
echo $BROWSERSTACK_ACCESS_KEY
```

Or check `.env.local` file exists and contains valid credentials.

### Benchmark function not found

Ensure:
1. Function has `#[benchmark]` attribute
2. Function is compiled into the mobile binary
3. Function name matches exactly (case-sensitive)

## Part of mobench

This CLI is part of the mobench ecosystem:

- **[mobench](https://crates.io/crates/mobench)** - This crate (CLI tool)
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - Core SDK with timing harness, build automation, and codegen
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - `#[benchmark]` proc macro

## See Also

- [mobench-sdk Documentation](https://crates.io/crates/mobench-sdk) for programmatic API
- [BrowserStack App Automate](https://www.browserstack.com/app-automate) for device cloud
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/) for FFI details

## License

Licensed under the MIT License. See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2026 World Foundation
