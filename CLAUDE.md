# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mobile-bench-rs (now **mobench**) is a mobile benchmarking SDK for Rust that enables developers to benchmark Rust functions on real Android and iOS devices via BrowserStack. It provides a library-first design with a `#[benchmark]` attribute macro and CLI tools for building, testing, and running benchmarks.

**Published on crates.io as the mobench ecosystem (v0.1.5):**
- **[mobench](https://crates.io/crates/mobench)** - CLI tool for mobile benchmarking
- **[mobench-sdk](https://crates.io/crates/mobench-sdk)** - Core SDK library with build automation
- **[mobench-macros](https://crates.io/crates/mobench-macros)** - `#[benchmark]` attribute proc macro
- **[mobench-runner](https://crates.io/crates/mobench-runner)** - Lightweight timing harness for mobile devices

All packages are licensed under MIT (World Foundation, 2026).

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
- **`crates/mobench-sdk`**: Core SDK library with registry system, builders (AndroidBuilder, IosBuilder), template generation, and BrowserStack integration.
- **`crates/mobench-macros`**: Proc macro crate providing the `#[benchmark]` attribute for marking functions.
- **`crates/mobench-runner`**: Lightweight timing harness library that gets embedded in mobile binaries. Provides timing infrastructure for benchmarks.
- **`examples/basic-benchmark`**: Example benchmark functions with UniFFI bindings for mobile platforms. Demonstrates the SDK usage pattern.

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

# Initialize SDK project
cargo mobench init --target android --output bench-config.toml

# Build mobile artifacts (recommended approach)
cargo mobench build --target android
cargo mobench build --target ios

# List discovered benchmarks
cargo mobench list

# Legacy: Direct script usage (for repository development only)
scripts/build-android-app.sh
scripts/build-ios.sh
```

## Common Commands

### Building with CLI (Recommended)

The `cargo mobench` CLI provides a unified build experience:

```bash
# Install the CLI
cargo install mobench

# Initialize project (generates config and scaffolding)
cargo mobench init --target android --output bench-config.toml

# Build for Android
cargo mobench build --target android

# Build for iOS
cargo mobench build --target ios

# Build for both platforms
cargo mobench build --target android
cargo mobench build --target ios

# Package iOS IPA (for BrowserStack or physical devices)
cargo mobench package-ipa --method adhoc
```

**What the CLI does:**
- Automatically builds Rust libraries with correct targets
- Generates or updates mobile app projects from embedded templates
- Syncs native libraries into platform-specific directories
- Builds APK (Android) or xcframework (iOS)
- No manual script execution needed

### Legacy Script-Based Building (Repository Development)

**Note:** The `scripts/` directory contains legacy tooling used for developing this repository. SDK users should use `cargo mobench build` instead.

#### Android (Legacy)
```bash
# Build Rust shared libraries for Android (requires Android NDK)
scripts/build-android.sh

# Sync .so files into Android project structure
scripts/sync-android-libs.sh

# Build complete APK with Gradle
cd android && gradle :app:assembleDebug

# Or use the all-in-one script
UNIFFI_ANDROID_ABI=x86_64 scripts/build-android-app.sh
```

Requirements:
- `ANDROID_NDK_HOME` environment variable set
- `cargo-ndk` installed: `cargo install cargo-ndk`
- Android SDK/NDK available (API level 24+)
- Set `UNIFFI_ANDROID_ABI=x86_64` for default Android Studio emulators

#### iOS (Legacy)
```bash
# Build Rust xcframework for iOS (includes UniFFI headers and automatic signing)
scripts/build-ios.sh

# Generate Xcode project from project.yml (if using repository's iOS app)
cd ios/BenchRunner && xcodegen generate

# Open in Xcode
open BenchRunner.xcodeproj
```

Requirements:
- Xcode command-line tools
- Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
- `xcodegen` installed: `brew install xcodegen` (only for repository development)

**Important iOS Build Details:**

The `build-ios.sh` script creates an xcframework with the following structure:
```
target/ios/sample_fns.xcframework/
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

**Automatic Code Signing**: The build script automatically signs the xcframework with:
```bash
codesign --force --deep --sign - target/ios/sample_fns.xcframework
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
  --output run-summary.json
```

#### BrowserStack Run (Android)
```bash
# Set credentials
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"

# Run on real devices
cargo mobench run \
  --target android \
  --function sample_fns::checksum \
  --iterations 30 \
  --warmup 5 \
  --devices "Google Pixel 7-13.0" \
  --output run-summary.json
```

#### BrowserStack Run (iOS)
```bash
cargo mobench run \
  --target ios \
  --function sample_fns::fibonacci \
  --iterations 20 \
  --warmup 3 \
  --devices "iPhone 14-16" \
  --ios-app target/ios/BenchRunner.ipa \
  --ios-test-suite target/ios/BenchRunnerUITests.zip \
  --output run-summary.json
```

#### Using Config Files
```bash
# Generate starter config
cargo mobench init --output bench-config.toml --target android

# Generate device matrix
cargo mobench plan --output device-matrix.yaml

# Run with config
cargo mobench run --config bench-config.toml
```

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

### FFI Boundary (`examples/basic-benchmark`)

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
cargo build -p basic-benchmark

# Generate Kotlin + Swift bindings
./scripts/generate-bindings.sh
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

1. Add function to `examples/basic-benchmark/src/lib.rs`
2. Add function dispatch to `run_benchmark()` match statement (e.g., `"my_func" => run_closure(spec, || my_func())`)
3. If adding new FFI types, add proc macro attributes (`#[derive(uniffi::Record)]`, `#[uniffi::export]`, etc.)
4. Regenerate bindings: `./scripts/generate-bindings.sh`
5. Rebuild native libraries: `cargo mobench build --target <android|ios>` or use legacy scripts
6. Mobile apps will automatically use the updated bindings

**Note**: No UDL file needed! Proc macros automatically detect FFI types from Rust code.

### Target Architectures

- **Android**: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` (emulator)
- **iOS**: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator on M1+ Macs)

### XCFramework Structure

`scripts/build-ios.sh` manually constructs an xcframework (not using `xcodebuild -create-xcframework`) by creating framework slices for each target with proper Info.plist and module.modulemap files.

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

7. **Code Signing**: After building, the xcframework must be code-signed for Xcode to accept it: `codesign --force --deep --sign - target/ios/sample_fns.xcframework`

### Gradle Integration (Android)

The Android app expects `.so` files under `android/app/src/main/jniLibs/{abi}/libsample_fns.so`. The `sync-android-libs.sh` script copies them from `target/android/{abi}/release/` to the correct locations.

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
app = "target/ios/BenchRunner.ipa"
test_suite = "target/ios/BenchRunnerUITests.zip"
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
codesign --force --deep --sign - target/ios/sample_fns.xcframework
```

### Issue: "While building for iOS Simulator, no library for this platform was found"
**Root Cause**: Incorrect xcframework structure (frameworks at wrong path or incorrectly named).

**Solution**: Ensure `build-ios.sh` creates the correct structure with frameworks in subdirectories:
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
  - `src/main.rs`: CLI entry point with commands (init, build, run, fetch, etc.)
  - `src/browserstack.rs`: BrowserStack REST API client
- **`crates/mobench-sdk/`**: Core SDK library (published to crates.io)
  - `src/lib.rs`: Public API surface
  - `src/registry.rs`: Function discovery via `inventory` crate
  - `src/runner.rs`: Timing harness integration
  - `src/builders/android.rs`: Android build automation
  - `src/builders/ios.rs`: iOS build automation
  - `src/codegen.rs`: Template generation from embedded files
  - `templates/`: Embedded Android/iOS app templates (via `include_dir!`)
- **`crates/mobench-macros/`**: Proc macro crate (published to crates.io)
  - `src/lib.rs`: `#[benchmark]` attribute implementation
- **`crates/mobench-runner/`**: Timing harness (published to crates.io)
  - `src/lib.rs`: Core timing and reporting logic

### Example & Testing
- **`examples/basic-benchmark/`**: Example benchmark crate demonstrating SDK usage
  - `src/lib.rs`: Sample benchmark functions with UniFFI bindings
  - `src/bin/generate-bindings.rs`: Binding generation for Kotlin/Swift
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

### Legacy Build Scripts (Repository Development Only)
- **`scripts/build-android.sh`**: Builds Rust libs with cargo-ndk for Android targets
- **`scripts/build-ios.sh`**: Builds iOS xcframework with correct structure and code signing
- **`scripts/sync-android-libs.sh`**: Copies .so files into Android jniLibs structure
- **`scripts/generate-bindings.sh`**: Regenerates UniFFI bindings for Kotlin/Swift

**Note**: SDK users should use `cargo mobench build` instead of calling scripts directly.
