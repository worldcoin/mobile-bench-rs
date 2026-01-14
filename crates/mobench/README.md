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
- `android/` or `ios/` - Platform-specific app projects
- `bench-sdk.toml` - Configuration file
- `benches/example.rs` - Example benchmarks (with `--generate-examples`)

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

Local emulator/simulator:
```bash
cargo mobench run fibonacci_30 --local-only --iterations 50
```

On real devices via BrowserStack:
```bash
export BROWSERSTACK_USERNAME=your_username
export BROWSERSTACK_ACCESS_KEY=your_access_key

cargo mobench run fibonacci_30 \
  --devices "Pixel 7-13" \
  --iterations 100 \
  --warmup 10
```

## Commands

### `init` - Initialize Project

Create mobile benchmarking infrastructure:

```bash
cargo mobench init [OPTIONS]
```

**Options:**
- `--target <android|ios|both>` - Target platform (default: android)
- `--output <FILE>` - Config file path (default: bench-sdk.toml)

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
- `--profile <debug|release>` - Build profile (default: debug)

**Examples:**
```bash
# Build Android APK in release mode
cargo mobench build --target android --profile release

# Build iOS xcframework
cargo mobench build --target ios
```

**Outputs:**
- Android: `android/app/build/outputs/apk/debug/app-debug.apk`
- iOS: `target/ios/sample_fns.xcframework`

### `run` - Run Benchmarks

Execute benchmarks on devices:

```bash
cargo mobench run <FUNCTION> [OPTIONS]
```

**Options:**
- `--target <android|ios>` - Platform (required)
- `--function <NAME>` - Benchmark function name (required)
- `--iterations <N>` - Number of iterations (default: 100)
- `--warmup <N>` - Warmup iterations (default: 10)
- `--devices <LIST>` - Comma-separated device list for BrowserStack
- `--local-only` - Skip BrowserStack, run locally only
- `--output <FILE>` - Save results to JSON file
- `--fetch` - Fetch BrowserStack results after completion

**Examples:**
```bash
# Run locally
cargo mobench run fibonacci_30 --target android --local-only

# Run on BrowserStack devices
cargo mobench run sha256_hash \
  --target android \
  --devices "Pixel 7-13,Galaxy S23-13" \
  --iterations 50 \
  --output results.json

# Run on iOS with auto-fetch
cargo mobench run json_parse \
  --target ios \
  --devices "iPhone 14-16,iPhone 15-17" \
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

**Output:** `target/ios/BenchRunner.ipa`

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
  - name: Pixel 7
    os: android
    os_version: "13.0"
    tags: [default, pixel]
  - name: iPhone 14
    os: ios
    os_version: "16"
    tags: [default, iphone]
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

## Configuration

### Config File Format (`bench-sdk.toml`)

```toml
[project]
name = "my-benchmarks"
target = "both"  # android, ios, or both

[build]
profile = "release"  # or "debug"

[browserstack]
username = "${BROWSERSTACK_USERNAME}"
access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "my-project-benchmarks"

[[devices]]
name = "Pixel 7"
os = "android"
os_version = "13.0"
tags = ["default"]

[[devices]]
name = "iPhone 14"
os = "ios"
os_version = "16"
tags = ["default"]
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
cargo mobench build --target android --profile release

# Run on multiple devices
cargo mobench run sha256_1kb \
  --target android \
  --devices "Pixel 7-13,Galaxy S23-13,OnePlus 11-13" \
  --iterations 200 \
  --output crypto-results.json
```

### Compare iOS Performance

```bash
# Run same benchmark on different iOS versions
cargo mobench run json_parse \
  --target ios \
  --devices "iPhone 13-15,iPhone 14-16,iPhone 15-17" \
  --iterations 100 \
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
        run: cargo mobench build --target android --profile release

      - name: Run benchmarks
        env:
          BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
          BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
        run: |
          cargo mobench run my_benchmark \
            --target android \
            --devices "Pixel 7-13" \
            --iterations 50 \
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
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - SDK library
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - Proc macros
- **[mobench-runner](https://crates.io/crates/mobench-runner)** - Timing harness

## See Also

- [mobench-sdk Documentation](https://crates.io/crates/mobench-sdk) for programmatic API
- [BrowserStack App Automate](https://www.browserstack.com/app-automate) for device cloud
- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/) for FFI details

## License

Licensed under the MIT License. See [LICENSE.md](../../LICENSE.md) for details.

Copyright (c) 2026 World Foundation
