# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mobile-bench-rs is a benchmarking tool for Rust functions on mobile devices (Android/iOS) using BrowserStack AppAutomate. It packages Rust functions into mobile binaries, runs them on real devices, and collects timing metrics.

## Core Architecture

### Workspace Structure

The repository is organized as a Cargo workspace with three main crates:

- **`mobench`**: CLI orchestrator that drives the entire workflow - building artifacts, uploading to BrowserStack, executing runs, and collecting results. Entry point for all operations.
- **`bench-runner`**: Lightweight harness library that gets embedded in mobile binaries. Provides timing infrastructure for benchmarks.
- **`sample-fns`**: Example benchmark functions with UniFFI bindings for mobile platforms. Compiled as `cdylib`, `staticlib`, and `rlib` for different mobile targets.

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
- Host testing (cargo test, CLI demo)
- Android testing (emulator, device, Android Studio; use `UNIFFI_ANDROID_ABI=x86_64` for default emulators)
- iOS testing (simulator, device, Xcode)
- Troubleshooting common issues
- Advanced testing scenarios

Quick test commands:
```bash
# Run all Rust tests
cargo test --all

# Test host harness
cargo run -p mobench -- demo --iterations 10 --warmup 2

# Android e2e (requires Android NDK)
scripts/build-android-app.sh
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity

# iOS e2e (requires Xcode)
scripts/build-ios.sh
cd ios/BenchRunner && xcodegen generate && open BenchRunner.xcodeproj
```

## Common Commands

### Building

#### Android
```bash
# Build Rust shared libraries for Android (requires Android NDK)
scripts/build-android.sh

# Sync .so files into Android project structure
scripts/sync-android-libs.sh

# Build complete APK with Gradle
cd android && gradle :app:assembleDebug

# Or use the full build script
scripts/build-android-app.sh
```

Requirements:
- `ANDROID_NDK_HOME` environment variable set
- `cargo-ndk` installed: `cargo install cargo-ndk`
- Android SDK/NDK available (API level 24+)

#### iOS
```bash
# Build Rust xcframework for iOS (includes UniFFI headers and automatic signing)
scripts/build-ios.sh

# Generate Xcode project from project.yml
cd ios/BenchRunner && xcodegen generate

# Open in Xcode
open BenchRunner.xcodeproj
```

Requirements:
- Xcode command-line tools
- Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
- `xcodegen` installed: `brew install xcodegen`

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

### Benchmarking

#### Local Smoke Test
```bash
cargo run -p mobench -- run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --local-only \
  --output run-summary.json
```

#### BrowserStack Run (Android)
```bash
cargo run -p mobench -- run \
  --target android \
  --function sample_fns::checksum \
  --iterations 30 \
  --warmup 5 \
  --devices "Pixel 7-13" \
  --output run-summary.json
```

#### BrowserStack Run (iOS)
```bash
cargo run -p mobench -- run \
  --target ios \
  --function sample_fns::fibonacci \
  --iterations 20 \
  --warmup 3 \
  --devices "iPhone 14-16" \
  --ios-app target/ios/BenchRunner.ipa \
  --ios-test-suite target/ios/BenchRunnerUITests.zip
```

#### Using Config Files
```bash
# Generate starter config
cargo run -p mobench -- init --output bench-config.toml --target android

# Generate device matrix
cargo run -p mobench -- plan --output device-matrix.yaml

# Run with config
cargo run -p mobench -- run --config bench-config.toml
```

## Key Implementation Details

### FFI Boundary (`sample-fns`)

This crate uses **UniFFI proc macros** to generate type-safe bindings for Kotlin and Swift. The API is defined directly in Rust code with attributes:

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

Regenerate bindings after modifying FFI types in `crates/sample-fns/src/lib.rs`:
```bash
# Build library to generate metadata
cargo build -p sample-fns

# Generate Kotlin + Swift bindings
./scripts/generate-bindings.sh
```

Generated files (committed to git):
- Kotlin: `android/app/src/main/java/uniffi/sample_fns/sample_fns.kt`
- Swift: `ios/BenchRunner/BenchRunner/Generated/sample_fns.swift`
- C header: `ios/BenchRunner/BenchRunner/Generated/sample_fnsFFI.h`

### Mobile Spec Injection

The CLI writes benchmark parameters to `target/mobile-spec/{android,ios}/bench_spec.json` during build. Mobile apps read this at runtime to know which function to benchmark.

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

### Adding New Benchmark Functions

1. Add function to `crates/sample-fns/src/lib.rs`
2. Add function dispatch to `run_benchmark()` match statement (e.g., `"my_func" => run_closure(spec, || my_func())`)
3. If adding new FFI types, add proc macro attributes (`#[derive(uniffi::Record)]`, `#[uniffi::export]`, etc.)
4. Regenerate bindings: `./scripts/generate-bindings.sh`
5. Rebuild native libraries: `scripts/build-android.sh` and/or `scripts/build-ios.sh`
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
  - name: Pixel 7
    os: android
    os_version: "13.0"
    tags: [default, pixel]
  - name: iPhone 14
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

- **`PROJECT_PLAN.md`**: Goals, architecture, task backlog
- **`TESTING.md`**: Comprehensive testing guide with detailed troubleshooting
- **`scripts/build-android.sh`**: Builds Rust libs with cargo-ndk for Android targets
- **`scripts/build-ios.sh`**: Builds iOS xcframework with correct structure and code signing
- **`scripts/sync-android-libs.sh`**: Copies .so files into Android jniLibs structure
- **`android/app/src/main/java/dev/world/bench/MainActivity.kt`**: Android app entry point
- **`ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift`**: iOS FFI wrapper
- **`ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h`**: Objective-C bridging header for C FFI types
- **`ios/BenchRunner/project.yml`**: XcodeGen project specification
- **`crates/mobench/src/browserstack.rs`**: BrowserStack REST API client
