# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mobile-bench-rs is a benchmarking tool for Rust functions on mobile devices (Android/iOS) using BrowserStack AppAutomate. It packages Rust functions into mobile binaries, runs them on real devices, and collects timing metrics.

## Core Architecture

### Workspace Structure

The repository is organized as a Cargo workspace with three main crates:

- **`bench-cli`**: CLI orchestrator that drives the entire workflow - building artifacts, uploading to BrowserStack, executing runs, and collecting results. Entry point for all operations.
- **`bench-runner`**: Lightweight harness library that gets embedded in mobile binaries. Provides timing infrastructure for benchmarks.
- **`sample-fns`**: Example benchmark functions with UniFFI bindings for mobile platforms. Compiled as `cdylib`, `staticlib`, and `rlib` for different mobile targets.

### Mobile Integration Flow

1. **Build Phase**: Rust functions are compiled to native libraries (`.so` for Android, `.a` for iOS)
2. **Bindings Generation**: UniFFI generates type-safe Kotlin/Swift bindings from `sample_fns.udl`
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
# Build Rust xcframework for iOS (uses UniFFI-generated headers)
scripts/build-ios.sh

# Generate Xcode project from project.yml
cd ios/BenchRunner && xcodegen generate
```

Requirements:
- Xcode command-line tools
- Rust targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
- `xcodegen` installed (for generating Xcode projects)

Note: UniFFI headers are generated automatically during the build process.

### Testing

```bash
# Run all host-side tests
cargo test --all

# Run local demo (exercises harness without mobile builds)
cargo run -p bench-cli -- demo --iterations 10 --warmup 2
```

### Benchmarking

#### Local Smoke Test
```bash
cargo run -p bench-cli -- run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --local-only \
  --output run-summary.json
```

#### BrowserStack Run (Android)
```bash
cargo run -p bench-cli -- run \
  --target android \
  --function sample_fns::checksum \
  --iterations 30 \
  --warmup 5 \
  --devices "Pixel 7-13" \
  --output run-summary.json
```

#### BrowserStack Run (iOS)
```bash
cargo run -p bench-cli -- run \
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
cargo run -p bench-cli -- init --output bench-config.toml --target android

# Generate device matrix
cargo run -p bench-cli -- plan --output device-matrix.yaml

# Run with config
cargo run -p bench-cli -- run --config bench-config.toml
```

### Android Device Testing

Launch the app with custom parameters via ADB:
```bash
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function sample_fns::checksum \
  --ei bench_iterations 30 \
  --ei bench_warmup 5
```

## Key Implementation Details

### FFI Boundary (`sample-fns`)

This crate uses **UniFFI** to generate type-safe bindings for Kotlin and Swift. The API is defined in `crates/sample-fns/src/sample_fns.udl`:

- **`runBenchmark(spec: BenchSpec) -> BenchReport`**: Main benchmark entrypoint with structured input/output
- **`BenchSpec`**: Struct containing `name` (function path), `iterations`, and `warmup` parameters
- **`BenchReport`**: Struct containing the original spec and a list of `BenchSample` timing results
- **`BenchError`**: Error enum with variants: `InvalidIterations`, `UnknownFunction`, `ExecutionFailed`

Regenerate bindings after modifying the UDL:
```bash
cargo run --bin generate-bindings --features bindgen
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
3. If the API surface changes (new public types or functions), update `sample_fns.udl`
4. Regenerate bindings: `cargo run --bin generate-bindings --features bindgen`
5. Rebuild native libraries: `scripts/build-android.sh` and/or `scripts/build-ios.sh`
6. Mobile apps will automatically use the updated bindings

### Target Architectures

- **Android**: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` (emulator)
- **iOS**: `aarch64-apple-ios` (device), `aarch64-apple-ios-sim` (simulator on M1+ Macs)

### XCFramework Structure

`scripts/build-ios.sh` manually constructs an xcframework (not using `xcodebuild -create-xcframework`) by creating framework slices for each target with proper Info.plist and module.modulemap files.

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

## Important Files

- **`PROJECT_PLAN.md`**: Goals, architecture, task backlog
- **`scripts/build-android.sh`**: Builds Rust libs with cargo-ndk for Android targets
- **`scripts/build-ios.sh`**: Builds iOS xcframework and generates C header
- **`scripts/sync-android-libs.sh`**: Copies .so files into Android jniLibs structure
- **`android/app/src/main/java/dev/world/bench/MainActivity.kt`**: Android app entry point
- **`ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift`**: iOS FFI wrapper
- **`crates/bench-cli/src/browserstack.rs`**: BrowserStack REST API client
