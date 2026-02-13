# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mobile-bench-rs (now **mobench**) is a mobile benchmarking SDK for Rust that enables developers to benchmark Rust functions on real Android and iOS devices via BrowserStack. It provides a library-first design with a `#[benchmark]` attribute macro and CLI tools for building, testing, and running benchmarks.

**Published on crates.io as the mobench ecosystem (v0.1.13):**

- **[mobench](https://crates.io/crates/mobench)** - CLI tool for mobile benchmarking
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - Core SDK library with timing harness and build automation
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - `#[benchmark]` attribute proc macro

All packages are licensed under MIT (World Foundation, 2026).

### Commit Guidelines

Do not add "Co-Authored-By" lines to commit messages.

### Quick Start (SDK Users)

```bash
# Install CLI
cargo install mobench

# Add to your project
cargo add mobench-sdk

# Mark functions for benchmarking
use mobench_sdk::benchmark;

#[benchmark]
fn my_benchmark() {
    // Your code here
}
```

## Core Architecture

### Workspace Structure

The repository is organized as a Cargo workspace:

- **`crates/mobench`**: CLI orchestrator that drives the entire workflow - building artifacts, uploading to BrowserStack, executing runs, and collecting results. Entry point for all operations.
- **`crates/mobench-sdk`**: Core SDK library with timing harness, registry system, builders (AndroidBuilder, IosBuilder), template generation, and BrowserStack integration. Includes the `timing` module for lightweight benchmarking (can be used standalone with `runner-only` feature).
- **`crates/mobench-macros`**: Proc macro crate providing the `#[benchmark]` attribute for marking functions.
- **`examples/basic-benchmark`**: Minimal SDK usage example with `#[benchmark]`.
- **`examples/ffi-benchmark`**: Full UniFFI surface example (types + `run_benchmark`).

### Mobile Integration Flow

1. **Build Phase**: Rust functions are compiled to native libraries (`.so` for Android, `.a` for iOS)
2. **Bindings Generation**: UniFFI **proc macros** generate type-safe Kotlin/Swift bindings from Rust code (no UDL file needed)
3. **Packaging**: Libraries and generated bindings are embedded into mobile apps (Android APK, iOS xcframework)
4. **Execution**: Apps read benchmark specs from:
   - Android: Intent extras or `bench_spec.json` asset
   - iOS: Environment variables, launch args, or `bench_spec.json` bundle resource
5. **FFI Boundary**: Mobile apps call `runBenchmark(spec)` via UniFFI-generated bindings which provide type-safe access to Rust code

### BrowserStack Integration

The CLI supports both Espresso (Android) and XCUITest (iOS) test automation frameworks:

- **Android**: Uploads app APK + test-suite APK (androidTest), schedules Espresso runs
- **iOS**: Uploads app IPA/bundle + XCUITest runner package, schedules XCUITest runs
- Credentials: `BROWSERSTACK_USERNAME`, `BROWSERSTACK_ACCESS_KEY` (from env or config)

## Build and Testing Documentation

**Primary Documentation:**

- **`BUILD.md`**: Complete build reference with prerequisites, step-by-step instructions, and troubleshooting for both Android and iOS
- **`TESTING.md`**: Comprehensive testing guide with advanced scenarios and detailed troubleshooting

For comprehensive testing instructions, see **`TESTING.md`** which includes:

- Prerequisites and setup
- Host testing (cargo test)
- Android testing (emulator, device, Android Studio; use `UNIFFI_ANDROID_ABI=x86_64` for default emulators)
- iOS testing (simulator, device, Xcode)
- Troubleshooting common issues
- Advanced testing scenarios

Quick test commands:

```bash
# Run all Rust tests
cargo test --all

# Check prerequisites before building (validates NDK, Xcode, Rust targets)
cargo mobench check --target android
cargo mobench check --target ios

# Initialize SDK project
cargo mobench init --target android --output bench-config.toml

# Build mobile artifacts (recommended approach)
# Outputs to target/mobench/ by default
cargo mobench build --target android
cargo mobench build --target ios

# Build with progress output for clearer feedback
cargo mobench build --target android --progress

# Build to custom output directory
cargo mobench build --target android --output-dir ./my-output

# List discovered benchmarks
cargo mobench list

# Verify benchmark setup (registry, spec, artifacts)
cargo mobench verify --target android --check-artifacts

# View benchmark result statistics
cargo mobench summary results.json

# List available BrowserStack devices
cargo mobench devices --platform android
```

## Common Commands

### Building with CLI (Recommended)

The `cargo mobench` CLI provides a unified build experience:

```bash
# Install the CLI
cargo install mobench

# Check prerequisites first (validates NDK, Xcode, Rust targets, etc.)
cargo mobench check --target android
cargo mobench check --target ios

# Initialize project (generates config and scaffolding)
cargo mobench init --target android --output bench-config.toml

# Build for Android
cargo mobench build --target android

# Build for iOS
cargo mobench build --target ios

# Build with simplified progress output
cargo mobench build --target android --progress
cargo mobench build --target ios --progress

# Build for both platforms
cargo mobench build --target android
cargo mobench build --target ios

# Package iOS IPA (for BrowserStack or physical devices)
cargo mobench package-ipa --method adhoc

# Package XCUITest runner (for BrowserStack iOS testing)
cargo mobench package-xcuitest

# Build in release mode (smaller artifacts, recommended for BrowserStack)
cargo mobench build --target android --release
cargo mobench build --target ios --release

# Verify build artifacts exist and are valid
cargo mobench verify --target android --check-artifacts
```

**What the CLI does:**

- Validates prerequisites with `check` command before building
- Automatically builds Rust libraries with correct targets
- Generates or updates mobile app projects from embedded templates
- Syncs native libraries into platform-specific directories
- Builds APK (Android) or xcframework (iOS)
- Outputs all artifacts to `target/mobench/` by default (use `--output-dir` to customize)
- Shows progress with `--progress` flag for clearer feedback
- No manual script execution needed

### Repository Development Builds

Use `cargo mobench build --target <android|ios>` for local or CI builds. The CLI handles
library builds, binding generation, and app packaging without extra scripts.

**Important iOS Build Details:**

The mobench iOS builder creates an xcframework with the following structure (default output directory is `target/mobench/`):

```
target/mobench/ios/sample_fns.xcframework/
├── Info.plist                           # XCFramework manifest
├── ios-arm64/                           # Device slice
│   └── sample_fns.framework/
│       ├── sample_fns                   # Static library (libsample_fns.a)
│       ├── Headers/
│       │   ├── sample_fnsFFI.h         # UniFFI-generated C header
│       │   └── module.modulemap        # Module map for Swift import
│       └── Info.plist
└── ios-simulator-arm64/                 # Simulator slice (M1+ Macs)
    └── sample_fns.framework/
        ├── sample_fns                   # Static library (libsample_fns.a)
        ├── Headers/
        │   ├── sample_fnsFFI.h
        │   └── module.modulemap
        └── Info.plist
```

**Key Configuration Details:**

- Framework binary must be named `sample_fns` (the module name), not the platform identifier
- Each framework slice must be in `{LibraryIdentifier}/sample_fns.framework/` directory structure
- Module map defines the C module as `sample_fnsFFI` (matches what UniFFI-generated Swift code imports)
- Info.plist uses `iPhoneOS`/`iPhoneSimulator` platform identifiers with `SupportedPlatformVariant`
- Framework bundle ID is `dev.world.sample-fns` (must not conflict with app bundle ID `dev.world.bench`)
- The Xcode project uses a bridging header (`BenchRunner-Bridging-Header.h`) to expose C FFI types to Swift
- UniFFI-generated Swift bindings are compiled directly into the app (no `import sample_fns` needed)

**Automatic Code Signing**: The build step automatically signs the xcframework with:

```bash
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework
```

If automatic signing fails, the script will display a warning with instructions for manual signing.

Note: UniFFI C headers are generated automatically during the build process and copied into each framework slice.

### Running Benchmarks

#### Local Testing (No BrowserStack)

```bash
# Build artifacts and write bench_spec.json (launch the app manually)
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --output target/mobench/results.json
```

#### Single-Command BrowserStack Flow

The `run` command provides a complete single-command workflow for benchmarking on real devices:

```bash
# Android: Single command does everything
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --devices "Google Pixel 7-13.0" \
  --release \
  --output target/mobench/results.json

# iOS: Single command also works (auto-packages IPA + XCUITest)
cargo mobench run \
  --target ios \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --devices "iPhone 14-16" \
  --release \
  --output target/mobench/results.json
```

**What happens automatically:**
1. Builds Rust native libraries for all required ABIs
2. Generates UniFFI bindings (Kotlin/Swift)
3. Packages mobile app (APK for Android, IPA for iOS)
4. Packages test runner (androidTest APK or XCUITest zip)
5. Uploads artifacts to BrowserStack
6. Schedules and monitors the test run
7. Fetches and displays results

**No need to manually call:**
- `cargo mobench build` (done automatically)
- `cargo mobench package-ipa` (done automatically for iOS)
- `cargo mobench package-xcuitest` (done automatically for iOS)

#### BrowserStack Run (Android)

```bash
# Set credentials
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"

# Run on real devices (use --release to reduce APK size for faster uploads)
cargo mobench run \
  --target android \
  --function sample_fns::checksum \
  --iterations 30 \
  --warmup 5 \
  --devices "Google Pixel 7-13.0" \
  --release \
  --output target/mobench/results.json
```

**Note on `--release` flag**: Debug builds can be very large (~544MB) which may cause BrowserStack upload timeouts. The `--release` flag builds in release mode, reducing APK size significantly (~133MB), and is recommended for all BrowserStack runs.

#### BrowserStack Run (iOS)

```bash
# First, package the IPA and XCUITest runner
cargo mobench package-ipa --method adhoc
cargo mobench package-xcuitest

# Run on real devices
cargo mobench run \
  --target ios \
  --function sample_fns::fibonacci \
  --iterations 20 \
  --warmup 3 \
  --devices "iPhone 14-16" \
  --release \
  --ios-app target/mobench/ios/BenchRunner.ipa \
  --ios-test-suite target/mobench/ios/BenchRunnerUITests.zip \
  --output target/mobench/results.json
```

#### Automatic iOS Packaging

When running iOS benchmarks on BrowserStack, mobench automatically packages the IPA and XCUITest runner if you don't provide `--ios-app` and `--ios-test-suite` flags:

```bash
# This auto-packages iOS artifacts:
cargo mobench run --target ios --function my_fn --devices "iPhone 14-16" --release

# Equivalent to manually running:
cargo mobench build --target ios --release
cargo mobench package-ipa --method adhoc
cargo mobench package-xcuitest
cargo mobench run --target ios --function my_fn --devices "iPhone 14-16" \
  --ios-app target/mobench/ios/BenchRunner.ipa \
  --ios-test-suite target/mobench/ios/BenchRunnerUITests.zip
```

You can override auto-packaging by providing both `--ios-app` and `--ios-test-suite` together.

#### Using Config Files

```bash
# Generate starter config
cargo mobench init --output bench-config.toml --target android

# Generate device matrix
cargo mobench plan --output device-matrix.yaml

# Run with config
cargo mobench run --config bench-config.toml
```

#### Release Builds for BrowserStack

Debug builds can be very large and may cause upload timeouts on BrowserStack:
- **Debug APK**: ~544MB
- **Release APK**: ~133MB

Always use the `--release` flag when running benchmarks on BrowserStack:

```bash
# Android - builds in release mode automatically
cargo mobench run --target android --release --devices "Google Pixel 7-13.0" ...

# iOS - builds in release mode automatically
cargo mobench run --target ios --release --devices "iPhone 14-16" ...
```

The `--release` flag ensures smaller artifact sizes and faster uploads.

#### Packaging iOS for BrowserStack

BrowserStack iOS testing requires two packages:
1. **App IPA**: The main application bundle
2. **XCUITest Runner**: The test automation package

```bash
# Package the app IPA
cargo mobench package-ipa --method adhoc

# Package the XCUITest runner for BrowserStack
cargo mobench package-xcuitest
```

The `package-xcuitest` command:
- Builds the XCUITest target from the iOS project
- Creates a properly structured zip file for BrowserStack
- Outputs to `target/mobench/ios/BenchRunnerUITests.zip`

#### Fetch BrowserStack Results

```bash
# Download results from previous run
cargo mobench fetch \
  --target android \
  --build-id abc123def456 \
  --output-dir ./results
```

## Key Implementation Details

### SDK Integration Pattern

Users import `mobench-sdk` and use the `#[benchmark]` macro:

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn my_expensive_operation() {
    let result = compute_something();
    std::hint::black_box(result);
}
```

The macro automatically registers functions at compile time via the `inventory` crate.

**Setup and Teardown (v0.1.13+)**: The `#[benchmark]` macro supports setup and teardown for excluding expensive initialization from timing:

```rust
// Setup runs once before all iterations (not measured)
fn setup_proof() -> ProofInput {
    generate_complex_proof()  // Expensive, but not timed
}

#[benchmark(setup = setup_proof)]
fn verify_proof(input: &ProofInput) {
    verify(&input.proof);  // Only this is measured
}

// Per-iteration setup for benchmarks that mutate input
fn generate_vec() -> Vec<i32> { (0..1000).collect() }

#[benchmark(setup = generate_vec, per_iteration)]
fn sort_benchmark(data: Vec<i32>) {
    let mut data = data;
    data.sort();  // Gets fresh data each iteration
}

// Setup + teardown for resources requiring cleanup
fn setup_db() -> Database { Database::connect("test.db") }
fn cleanup_db(db: Database) { db.close(); }

#[benchmark(setup = setup_db, teardown = cleanup_db)]
fn db_query(db: &Database) {
    db.query("SELECT *");
}
```

**Macro Validation (v0.1.13+)**: The `#[benchmark]` macro validates function signatures at compile time:
- Simple benchmarks: no parameters, returns `()`
- With setup: one parameter matching setup return type
- Compile errors include helpful messages about requirements

**Debugging Benchmark Registration**: Use the `debug_benchmarks!()` macro to verify benchmarks are properly registered:

```rust
use mobench_sdk::{benchmark, debug_benchmarks};

#[benchmark]
fn my_benchmark() {
    std::hint::black_box(42);
}

// Generate debug function
debug_benchmarks!();

fn main() {
    // Print all registered benchmarks
    _debug_print_benchmarks();
    // Output:
    // Discovered benchmarks:
    //   - my_crate::my_benchmark
}
```

If no benchmarks appear, check:
1. Functions are annotated with `#[benchmark]`
2. Functions are `pub` (public visibility)
3. Simple benchmarks: no parameters, returns `()`
4. Setup benchmarks: one parameter matching setup return type
5. The `inventory` crate is in your dependencies

### FFI Boundary (`examples/ffi-benchmark`)

The example crate uses **UniFFI proc macros** to generate type-safe bindings for Kotlin and Swift. The API is defined directly in Rust code with attributes:

```rust
#[derive(uniffi::Record)]
pub struct BenchSpec {
    pub name: String,
    pub iterations: u32,
    pub warmup: u32,
}

#[derive(uniffi::Error)]
pub enum BenchError {
    InvalidIterations,
    UnknownFunction { name: String },
    ExecutionFailed { reason: String },
}

#[uniffi::export]
pub fn run_benchmark(spec: BenchSpec) -> Result<BenchReport, BenchError> {
    // Implementation
}

uniffi::setup_scaffolding!();  // Auto-uses crate name as namespace
```

Regenerate bindings after modifying FFI types (for repository development):

```bash
# Build library to generate metadata
cargo build -p ffi-benchmark

# Generate Kotlin + Swift bindings
cargo mobench build --target android
```

Generated files (committed to git for the example app):

- Kotlin: `android/app/src/main/java/uniffi/sample_fns/sample_fns.kt`
- Swift: `ios/BenchRunner/BenchRunner/Generated/sample_fns.swift`
- C header: `ios/BenchRunner/BenchRunner/Generated/sample_fnsFFI.h`

### Template System

The SDK embeds Android and iOS app templates using the `include_dir!` macro:

- Templates located in `crates/mobench-sdk/templates/`
- Embedded at compile time (no runtime file access needed)
- Generated projects are created in user's workspace via `cargo mobench init`

### Mobile Spec Injection

The CLI writes benchmark parameters to `target/mobile-spec/{android,ios}/bench_spec.json` during build. Mobile apps read this at runtime to know which function to benchmark.

When using SDK-generated projects:

- Templates include spec reading logic
- Apps automatically parse `bench_spec.json` from assets/bundle
- Supports runtime parameter override via Intent extras (Android) or environment variables (iOS)

### BrowserStack Credentials

Credentials are resolved in this order:

1. Config file (supports `${ENV_VAR}` expansion)
2. Environment variables: `BROWSERSTACK_USERNAME`, `BROWSERSTACK_ACCESS_KEY`, `BROWSERSTACK_PROJECT`
3. `.env.local` file (loaded automatically via `dotenvy`)

**Improved Error Messages (v0.1.13+)**: Missing credentials now show setup instructions:
- Instructions for setting environment variables
- Link to BrowserStack account settings page
- Hints for `.env.local` file setup

**Device Validation**: Use `cargo mobench devices` to list and validate device specs:

```bash
# List all available devices
cargo mobench devices

# Filter by platform
cargo mobench devices --platform android
cargo mobench devices --platform ios

# Validate device specs
cargo mobench devices --validate "Google Pixel 7-13.0"

# Output as JSON
cargo mobench devices --json
```

Invalid device specs now show suggestions for similar devices.

### CI/CD (`.github/workflows/mobile-bench.yml`)

The workflow supports manual dispatch with platform selection:

- Runs host tests first
- Builds Android APK and/or iOS xcframework
- Uploads artifacts
- Optionally triggers BrowserStack runs (requires secrets)

## Development Notes

### Using mobench-sdk in Your Project

1. Add dependencies to your `Cargo.toml`:

```toml
[dependencies]
mobench-sdk = "0.1"
inventory = "0.3"
```

1. Mark functions with `#[benchmark]`:

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn my_function() {
    // Your code
    std::hint::black_box(result);
}
```

1. Build for mobile:

```bash
cargo mobench build --target android
cargo mobench build --target ios
```

1. Run benchmarks:

```bash
cargo mobench run --target android --function my_function
```

### Adding New Benchmark Functions to Repository Example

1. Add function to `crates/sample-fns/src/lib.rs`
2. Add function dispatch to `run_benchmark()` match statement (e.g., `"my_func" => run_closure(spec, || my_func())`)
3. If adding new FFI types, add proc macro attributes (`#[derive(uniffi::Record)]`, `#[uniffi::export]`, etc.)
4. Regenerate bindings: `cargo mobench build --target android`
5. Rebuild native libraries: `cargo mobench build --target <android|ios>`
6. Mobile apps will automatically use the updated bindings

**Note**: No UDL file needed! Proc macros automatically detect FFI types from Rust code.

### Target Architectures

- **Android**: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` (emulator)
- **iOS**: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator on M1+ Macs)

### XCFramework Structure

The mobench iOS builder manually constructs an xcframework (not using `xcodebuild -create-xcframework`) by creating framework slices for each target with proper Info.plist and module.modulemap files.

**Critical Implementation Details:**

1. **Directory Structure**: Each framework must be in `{LibraryIdentifier}/{FrameworkName}.framework/`, not directly at the root. For example: `ios-simulator-arm64/sample_fns.framework/`, not `ios-simulator-arm64.framework/`.

2. **Framework Binary Naming**: The binary inside each framework slice must be named after the module (`sample_fns`), not the platform identifier (`ios-simulator-arm64`). This is what Xcode's linker expects.

3. **Module Map**: The C module in `module.modulemap` must be named `sample_fnsFFI` to match what UniFFI-generated Swift code tries to import via `#if canImport(sample_fnsFFI)`.

4. **Platform Identifiers**: The framework Info.plist uses Apple's official platform names:

   - Device: `CFBundleSupportedPlatforms = ["iPhoneOS"]`
   - Simulator: `CFBundleSupportedPlatforms = ["iPhoneSimulator"]`

   The xcframework Info.plist uses `SupportedPlatform = "ios"` with `SupportedPlatformVariant = "simulator"` for simulator slices.

5. **Bundle Identifier**: The framework bundle ID must not conflict with the app's bundle ID. Use `dev.world.sample-fns` for the framework, while the app uses `dev.world.bench`.

6. **Static vs Dynamic**: The xcframework contains static libraries (`.a` archives built with `staticlib` crate-type), not dynamic frameworks. This requires a bridging header in the Xcode project to expose C types to Swift.

7. **Code Signing**: After building, the xcframework must be code-signed for Xcode to accept it: `codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework`

### Gradle Integration (Android)

The Android app expects `.so` files under `target/mobench/android/app/src/main/jniLibs/{abi}/libsample_fns.so`. The mobench Android builder copies them from `target/{abi}/release/` to the correct locations.

## Configuration Files

### `bench-config.toml` (generated by `init` command)

```toml
target = "android"
function = "sample_fns::fibonacci"
iterations = 100
warmup = 10
device_matrix = "device-matrix.yaml"

[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "mobile-bench-rs"

# iOS only:
[ios_xcuitest]
app = "target/mobench/ios/BenchRunner.ipa"
test_suite = "target/mobench/ios/BenchRunnerUITests.zip"
```

### `device-matrix.yaml` (generated by `plan` command)

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

## Common iOS Build Issues and Solutions

### Issue: "The Framework 'sample_fns.xcframework' is unsigned"

**Solution**: Code-sign the xcframework after building:

```bash
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework
```

### Issue: "While building for iOS Simulator, no library for this platform was found"

**Root Cause**: Incorrect xcframework structure (frameworks at wrong path or incorrectly named).

**Solution**: Ensure the iOS builder creates the correct structure with frameworks in subdirectories:

```
ios-simulator-arm64/sample_fns.framework/  (not ios-simulator-arm64.framework/)
```

### Issue: "framework 'ios-simulator-arm64' not found" (linker error)

**Root Cause**: Framework LibraryPath in xcframework Info.plist points to wrong name.

**Solution**: Verify xcframework Info.plist has:

```xml
<key>LibraryPath</key>
<string>sample_fns.framework</string>  <!-- NOT ios-simulator-arm64.framework -->
```

### Issue: "Unable to find module dependency: 'sample_fns'" in Swift

**Root Cause**: Trying to import the module when it should be compiled directly into the app.

**Solution**: Remove `import sample_fns` from Swift files. The UniFFI-generated Swift bindings are compiled into the app target, and C types are exposed via the bridging header.

### Issue: "Cannot find type 'RustBuffer' in scope"

**Root Cause**: Bridging header missing or not configured.

**Solution**:

1. Ensure `BenchRunner-Bridging-Header.h` exists with `#import "sample_fnsFFI.h"`
2. Verify `project.yml` has `SWIFT_OBJC_BRIDGING_HEADER` set
3. Regenerate Xcode project: `xcodegen generate`

### Issue: "Framework had an invalid CFBundleIdentifier"

**Root Cause**: Framework bundle ID conflicts with app bundle ID.

**Solution**: Use different bundle IDs:

- Framework: `dev.world.sample-fns`
- App: `dev.world.bench`

## Important Files

### Core SDK Crates

- **`crates/mobench/`**: CLI tool (published to crates.io)
  - `src/lib.rs`: CLI entry point with commands (init, build, run, fetch, check, verify, summary, devices, etc.)
  - `src/main.rs`: CLI binary wrapper
  - `src/browserstack.rs`: BrowserStack REST API client
  - `src/config.rs`: Configuration file support for `mobench.toml`
- **`crates/mobench-sdk/`**: Core SDK library (published to crates.io)
  - `src/lib.rs`: Public API surface
  - `src/timing.rs`: Lightweight timing harness (BenchSpec, BenchReport, run_closure)
  - `src/registry.rs`: Function discovery via `inventory` crate
  - `src/runner.rs`: Benchmark execution engine using timing module
  - `src/builders/android.rs`: Android build automation
  - `src/builders/ios.rs`: iOS build automation
  - `src/codegen.rs`: Template generation from embedded files
  - `templates/`: Embedded Android/iOS app templates (via `include_dir!`)
- **`crates/mobench-macros/`**: Proc macro crate (published to crates.io)
  - `src/lib.rs`: `#[benchmark]` attribute implementation

### Example & Testing

- **`examples/basic-benchmark/`**: Minimal SDK usage example
- **`examples/ffi-benchmark/`**: Full UniFFI surface example
- **`android/`**: Android test app (for repository development)
  - `app/src/main/java/dev/world/bench/MainActivity.kt`: Android app entry point
  - `app/src/main/java/uniffi/sample_fns/sample_fns.kt`: Generated Kotlin bindings
- **`ios/BenchRunner/`**: iOS test app (for repository development)
  - `BenchRunner/BenchRunnerFFI.swift`: iOS FFI wrapper
  - `BenchRunner/BenchRunner-Bridging-Header.h`: Objective-C bridging header
  - `BenchRunner/Generated/`: UniFFI-generated Swift bindings and C headers
  - `project.yml`: XcodeGen project specification

### Documentation

- **`BUILD.md`**: Complete build reference with prerequisites and troubleshooting
- **`TESTING.md`**: Comprehensive testing guide with detailed troubleshooting
- **`BENCH_SDK_INTEGRATION.md`**: Integration guide for SDK users
- **`PROJECT_PLAN.md`**: Goals, architecture, task backlog
- **`CLAUDE.md`**: This file - developer guide for the codebase

### Build Tooling

Use `cargo mobench build --target <android|ios>` for repository development and CI. The CLI
handles native builds, binding generation, and packaging.
