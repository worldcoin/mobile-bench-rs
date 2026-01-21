# mobench-sdk Integration Guide

This guide shows how to integrate `mobench-sdk` into an existing Rust project, run local
mobile benchmarks, and then run them on BrowserStack.

> **Important**: This guide is for integrators importing `mobench-sdk` as a library.
> All build functionality is available via `cargo mobench` commands.

## Quick Setup Checklist

Before diving into the full guide, ensure your project meets these requirements:

### Required Cargo.toml entries

```toml
[dependencies]
mobench-sdk = "0.1"
inventory = "0.3"  # Required for benchmark registration

[lib]
# Required for mobile FFI - produces .so (Android) and .a (iOS)
crate-type = ["cdylib", "staticlib", "lib"]
```

### Benchmark Function Requirements

Functions marked with `#[benchmark]` must:
- Take **no parameters** (setup should be inside the function)
- Return **()** (unit type) - use `std::hint::black_box()` for results
- Be **public** (`pub fn`)

```rust
use mobench_sdk::benchmark;

// CORRECT - no params, returns ()
#[benchmark]
pub fn my_benchmark() {
    let input = create_input();  // Setup inside
    let result = compute(input);
    std::hint::black_box(result);  // Consume result
}

// WRONG - has parameters (compile error)
#[benchmark]
pub fn bad_benchmark(data: &[u8]) { ... }

// WRONG - returns a value (compile error)
#[benchmark]
pub fn bad_benchmark() -> u64 { 42 }
```

### Verify Your Setup

After adding benchmarks, verify everything is working:

```bash
# Check prerequisites are installed
cargo mobench check --target android

# List discovered benchmarks
cargo mobench list

# Verify registry, spec, and artifacts
cargo mobench verify --smoke-test --function my_crate::my_benchmark
```

## 1) Prerequisites

Install the following tools (per platform):

- Rust toolchain (stable) + `rustup`:
  - https://www.rust-lang.org/tools/install
- Android:
  - Android Studio (SDK + NDK manager): https://developer.android.com/studio
  - Android NDK (API 24+): https://developer.android.com/ndk/downloads
  - `cargo-ndk` (`cargo install cargo-ndk`): https://github.com/bbqsrc/cargo-ndk
  - JDK 17+ (for Gradle; any distribution): https://openjdk.org/install/
    - Note: Android Gradle Plugin (AGP) officially supports Java 17.
- iOS (macOS only):
  - Xcode + Command Line Tools: https://developer.apple.com/xcode/
  - Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
    - https://doc.rust-lang.org/rustup/targets.html
  - `xcodegen` (optional): https://github.com/yonaskolb/XcodeGen

## 2) Add mobench-sdk to your crate

In your project's `Cargo.toml`:

```toml
[dependencies]
mobench-sdk = "0.1"
```

## 3) Annotate benchmark functions

Add `#[mobench_sdk::benchmark]` to any function you want to run on devices.

```rust
use mobench_sdk::benchmark;

#[benchmark]
pub fn checksum_bench() {
    let data = [1u8; 1024];
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

### Macro Validation

The `#[benchmark]` macro validates function signatures at compile time:

**No parameters allowed:**
```rust
// ERROR: #[benchmark] functions must take no parameters.
// Found 1 parameter(s): data: &[u8]
#[benchmark]
pub fn bad_benchmark(data: &[u8]) { ... }
```

**Must return unit type:**
```rust
// ERROR: #[benchmark] functions must return () (unit type).
// Found return type: u64
#[benchmark]
pub fn bad_benchmark() -> u64 { 42 }
```

The compiler provides helpful suggestions for fixing these issues.

### Debugging Registration Issues

If benchmarks aren't being discovered, use the `debug_benchmarks!()` macro:

```rust
use mobench_sdk::{benchmark, debug_benchmarks};

#[benchmark]
pub fn my_benchmark() {
    std::hint::black_box(42);
}

// Generate the debug function
debug_benchmarks!();

fn main() {
    // Print all registered benchmarks
    _debug_print_benchmarks();
    // Output:
    // Discovered benchmarks:
    //   - my_crate::my_benchmark
}
```

If no benchmarks are printed, the macro provides troubleshooting tips:
1. Ensure functions are annotated with `#[benchmark]`
2. Ensure functions are `pub` (public visibility)
3. Ensure the crate with benchmarks is linked into the binary
4. Check that `inventory` crate is in your dependencies

Benchmarks are identified by name at runtime. You can call them by:

- Fully-qualified path (e.g., `my_crate::checksum_bench`)
- Or suffix match (e.g., `checksum_bench`)

## 4) Scaffold mobile projects

From your repo root, create a mobile harness with the CLI:

```bash
cargo mobench init --target android --output bench-config.toml
```

This generates:

- `bench-mobile/` (FFI bridge that links your crate)
- `android/` and `ios/` app templates
- `bench-config.toml` configuration

## 5) Local Android testing

Build the Android app using the mobench:

```bash
cargo mobench build --target android
```

This automatically:

- Builds Rust libraries for all Android ABIs (arm64-v8a, armeabi-v7a, x86_64)
- Generates UniFFI Kotlin bindings
- Copies .so files to jniLibs
- Runs Gradle to create the APK

Install and run on emulator or device:

```bash
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity
```

To override benchmark parameters:

```bash
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function my_crate::checksum_bench \
  --ei bench_iterations 30 \
  --ei bench_warmup 5
```

## 6) Local iOS testing

Build the iOS xcframework using the mobench:

```bash
cargo mobench build --target ios
```

This automatically:

- Builds Rust libraries for iOS device + simulator
- Generates UniFFI Swift bindings and C headers
- Creates properly structured xcframework
- Code-signs the framework
- Generates Xcode project (if xcodegen is installed)

Open and run in Xcode:

```bash
open ios/BenchRunner/BenchRunner.xcodeproj
```

The app will read `bench_spec.json` from the bundle or use defaults.

## 7) BrowserStack (Android Espresso)

Build APK + test APK:

```bash
cargo mobench build --target android
cd android
./gradlew :app:assembleDebugAndroidTest
cd ..
```

Run on BrowserStack:

```bash
cargo mobench run \
  --target android \
  --function my_crate::checksum_bench \
  --iterations 100 \
  --warmup 10 \
  --devices "Google Pixel 7-13.0" \
  --release
```

**Important**: Always use the `--release` flag for BrowserStack runs. Debug builds are significantly larger (~544MB vs ~133MB for release) and may cause upload timeouts.

The CLI will automatically:

- Build in release mode (with `--release` flag)
- Upload APK and test APK to BrowserStack
- Schedule the test run
- Wait for completion
- Download results and logs

## 8) BrowserStack (iOS XCUITest)

Build iOS artifacts and package for BrowserStack:

```bash
# Build xcframework
cargo mobench build --target ios

# Package as IPA (ad-hoc signing, no Apple ID needed)
cargo mobench package-ipa --method adhoc

# Package the XCUITest runner for BrowserStack
cargo mobench package-xcuitest

# Or for development signing (requires Apple Developer account)
cargo mobench package-ipa --method development
```

Run on BrowserStack:

```bash
cargo mobench run \
  --target ios \
  --function my_crate::checksum_bench \
  --iterations 100 \
  --warmup 10 \
  --devices "iPhone 14-16" \
  --release \
  --ios-app target/mobench/ios/BenchRunner.ipa \
  --ios-test-suite target/mobench/ios/BenchRunnerUITests.zip
```

**Important**: Always use the `--release` flag for BrowserStack runs to reduce artifact sizes and prevent upload timeouts.

**iOS Packaging Commands:**

- `package-ipa`: Creates the app IPA bundle for device deployment
  - `--method adhoc`: No Apple ID required, works for BrowserStack
  - `--method development`: Requires Apple Developer account
- `package-xcuitest`: Creates the XCUITest runner zip that BrowserStack uses to drive test automation. Outputs to `target/mobench/ios/BenchRunnerUITests.zip`

## 9) Verification and Troubleshooting

### Check Prerequisites

Before building, verify all required tools are installed:

```bash
# Check Android prerequisites
cargo mobench check --target android

# Check iOS prerequisites
cargo mobench check --target ios

# Output as JSON for CI
cargo mobench check --target android --format json
```

### Verify Benchmark Setup

Use the `verify` command to validate your setup:

```bash
# Full verification with smoke test
cargo mobench verify --target android --check-artifacts --smoke-test --function my_crate::my_benchmark

# Check specific spec file
cargo mobench verify --spec-path target/mobench/android/app/src/main/assets/bench_spec.json
```

The verify command checks:
1. Benchmark registry has functions registered
2. Spec file exists and is valid
3. Build artifacts are present
4. Optional smoke test passes

### View Result Summaries

After running benchmarks, get statistics with the `summary` command:

```bash
# Text summary (default)
cargo mobench summary results.json

# JSON format
cargo mobench summary results.json --format json

# CSV format
cargo mobench summary results.json --format csv
```

### Common Errors and Solutions

**"unknown benchmark function":**
```
Error: unknown benchmark function: 'my_func'. Available benchmarks: ["other_func"]

Ensure the function is:
  1. Annotated with #[benchmark]
  2. Public (pub fn)
  3. Takes no parameters and returns ()
```

**"iterations must be greater than zero":**
```
Error: iterations must be greater than zero (got 0). Minimum recommended: 10
```

**Benchmark not discovered:**
- Use `debug_benchmarks!()` macro to debug
- Verify function is `pub` and annotated with `#[benchmark]`
- Ensure `inventory` crate is a dependency

## Notes

- **No scripts needed**: All functionality is available via `cargo mobench` commands
- **Use `--release` for BrowserStack**: Debug builds are ~544MB, release builds are ~133MB. Large artifacts can cause upload timeouts.
- **Validate before running**: Use `cargo mobench verify` to catch issues early
- If you change FFI types, the build process automatically regenerates bindings
- Android emulator ABI is typically `x86_64` in Android Studio
- BrowserStack credentials must be set via `BROWSERSTACK_USERNAME` and `BROWSERSTACK_ACCESS_KEY`
- For repository development, use the same `cargo mobench` workflow
