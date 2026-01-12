# mobile-bench-rs
Benchmarking tool for Rust functions on mobile devices using BrowserStack.

## Layout
- `crates/bench-cli`: CLI orchestrator for building/packaging benchmarks and driving BrowserStack runs (stubbed).
- `crates/bench-runner`: Shared harness that will be embedded in Android/iOS binaries; currently host-side only.
- `crates/sample-fns`: Small Rust functions used as demo benchmarks with UniFFI bindings for mobile platforms.
- `PROJECT_PLAN.md`: Goals, architecture outline, and initial task backlog.
- `android/`: Minimal Android app that loads the Rust demo library; Gradle project for BrowserStack/AppAutomate runs.

## Quick Start

### Host Demo (No Mobile Build Required)
Test the benchmarking harness locally:
```bash
cargo run -p bench-cli -- demo --iterations 10 --warmup 2
```

### Mobile Testing
For complete end-to-end testing on Android/iOS, see the **[End-to-End Testing](#end-to-end-testing)** section below.

**Quick commands:**
- **Android**: `scripts/build-android-app.sh` then install APK
- **iOS**: `scripts/build-ios.sh` then open in Xcode

### Generate Config Files
```bash
cargo run -p bench-cli -- init --output bench-config.toml
cargo run -p bench-cli -- plan --output device-matrix.yaml
```

## UniFFI Bindings

This project uses [UniFFI](https://mozilla.github.io/uniffi-rs/) to generate type-safe Kotlin and Swift bindings from Rust.

To regenerate bindings after modifying the API (`crates/sample-fns/src/sample_fns.udl`):
```bash
cargo run --bin generate-bindings --features bindgen
```

Generated files (committed to git for reproducibility):
- **Kotlin**: `android/app/src/main/java/uniffi/sample_fns/sample_fns.kt`
- **Swift**: `ios/BenchRunner/BenchRunner/Generated/sample_fns.swift`
- **C header**: `ios/BenchRunner/BenchRunner/Generated/sample_fnsFFI.h`

The UniFFI API exposes:
- `runBenchmark(spec: BenchSpec) -> BenchReport`: Run a benchmark by name
- `BenchSpec(name, iterations, warmup)`: Benchmark configuration
- `BenchReport`: Contains timing samples and statistics
- `BenchError`: Type-safe error handling (InvalidIterations, UnknownFunction, ExecutionFailed)

## End-to-End Testing

### Android Testing

#### Quick Start (All-in-One)
```bash
# Build everything and create APK
scripts/build-android-app.sh

# Install and launch on emulator/device
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity
```

#### Step-by-Step
```bash
# 1. Build Rust libraries for all Android ABIs (arm64-v8a, armeabi-v7a, x86_64)
scripts/build-android.sh

# 2. Sync .so files into Android project structure (jniLibs)
scripts/sync-android-libs.sh

# 3. Build the APK with Gradle
cd android && ./gradlew :app:assembleDebug

# 4. Install and launch
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity
```

#### Testing with Custom Parameters
```bash
# Launch with custom benchmark function and parameters
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function sample_fns::checksum \
  --ei bench_iterations 30 \
  --ei bench_warmup 5
```

#### Using Android Studio
1. Open the `android/` directory in Android Studio
2. Ensure Rust libraries are built: `scripts/build-android.sh`
3. Sync libs: `scripts/sync-android-libs.sh`
4. Click Run (the app module should auto-sync)
5. Select emulator/device and run

**Expected Output**: The app displays formatted benchmark results with individual sample timings and statistics (min/max/avg).

### iOS Testing

#### Prerequisites
```bash
# Install xcodegen if not already installed
brew install xcodegen

# Install Rust iOS targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
```

#### Step-by-Step
```bash
# 1. Build Rust xcframework for iOS (includes UniFFI headers and automatic code signing)
scripts/build-ios.sh

# The script creates a properly structured xcframework with:
# - Static libraries for device (aarch64-apple-ios) and simulator (aarch64-apple-ios-sim)
# - UniFFI-generated C headers in each framework slice
# - Module maps for Swift interop
# - Correct bundle identifiers and platform identifiers
# - Automatic code signing for Xcode compatibility

# 2. Generate Xcode project from project.yml
cd ios/BenchRunner
xcodegen generate

# 3. Open in Xcode
open BenchRunner.xcodeproj
```

Then in Xcode:
1. Select a simulator (e.g., iPhone 15) or connected device
2. Click Run (⌘R) or Product → Run
3. The app will display benchmark results

**Note**: The project uses a bridging header to expose C FFI types from Rust to Swift. The UniFFI-generated Swift bindings are compiled directly into the app (no module import needed).

#### Testing with Custom Parameters

**Method 1: Edit Scheme (Xcode)**
1. Product → Scheme → Edit Scheme...
2. Run → Arguments → Environment Variables
3. Add variables:
   - `BENCH_FUNCTION` = `sample_fns::checksum`
   - `BENCH_ITERATIONS` = `30`
   - `BENCH_WARMUP` = `5`
4. Run the app

**Method 2: Command Line (simulator only)**
```bash
# Build and run with xcrun
xcrun simctl launch booted dev.world.bench.BenchRunner \
  --bench-function=sample_fns::checksum \
  --bench-iterations=30 \
  --bench-warmup=5
```

**Expected Output**: The app displays formatted benchmark results with individual sample timings and statistics (min/max/avg).

### Key Differences from Pre-UniFFI

The build process is **simpler** now:
- ✅ No need to run `cbindgen` manually
- ✅ UniFFI headers (`sample_fnsFFI.h`) are automatically generated during `build-ios.sh`
- ✅ Kotlin/Swift bindings are already committed to git
- ✅ Only regenerate bindings if you change `sample_fns.udl` (via `cargo run --bin generate-bindings --features bindgen`)
- ✅ Apps show formatted output with statistics instead of raw JSON
- ✅ Type-safe error handling (no more string parsing)

### Requirements

**Android:**
- Android SDK/NDK (API level 24+)
- `ANDROID_NDK_HOME` environment variable set
- `cargo-ndk` installed: `cargo install cargo-ndk`
- Android emulator or physical device

**iOS:**
- macOS with Xcode command-line tools
- Rust targets: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim`
- `xcodegen` installed: `brew install xcodegen`
- iOS Simulator or physical device (requires code signing)

## Additional Documentation

- **`BUILD.md`**: Complete build reference guide for Android and iOS (prerequisites, step-by-step instructions, troubleshooting)
- **`TESTING.md`**: Comprehensive testing guide with troubleshooting and advanced scenarios
- **`PROJECT_PLAN.md`**: Project goals, architecture, and task backlog
- **`CLAUDE.md`**: Developer guide for working with this codebase (for Claude Code and developers)

## BrowserStack XCUITest (iOS)
- Provide signed artifacts for BrowserStack real devices: the app bundle (`.ipa` or zipped `.app`) and the XCUITest runner package (`.zip` or `.ipa` containing the test bundle).
- CLI flags (when not using a config file): `--ios-app` and `--ios-test-suite` must both be set whenever `--target ios` is paired with `--devices`.
- Config block example:
  ```toml
  [ios_xcuitest]
  app = "target/ios/BenchRunner.ipa"
  test_suite = "target/ios/BenchRunnerUITests.zip"
  ```
- Example run (requires BrowserStack credentials in env vars): `cargo run -p bench-cli -- run --target ios --function sample_fns::checksum --iterations 10 --warmup 2 --devices "iPhone 14-16" --ios-app target/ios/BenchRunner.ipa --ios-test-suite target/ios/BenchRunnerUITests.zip`
