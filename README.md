# mobile-bench-rs → mobench-sdk

**Mobile benchmarking SDK for Rust** - Run Rust benchmarks on real Android and iOS devices.

> **Phase 1 MVP Complete!** This project has been transformed into an importable library crate (`mobench-sdk`) that can be published to crates.io.

## 🎯 For SDK Integrators

**Importing mobench-sdk into your project?** You do **NOT** need the `scripts/` directory!

- ✅ Use `cargo mobench build --target <android|ios|both>` for all builds
- ✅ All build logic is in pure Rust (no shell scripts required)
- ✅ Templates are embedded in the binary
- ✅ See **[BENCH_SDK_INTEGRATION.md](BENCH_SDK_INTEGRATION.md)** for the integration guide

**The `scripts/` directory** is legacy tooling for developing this repository. SDK users should ignore it.

---

## 🚀 What's New in Phase 1

### Library-First Design

Use `mobench-sdk` in any Rust project:

```toml
[dependencies]
mobench-sdk = "0.1"
inventory = "0.3"  # Required for registry
```

### #[benchmark] Macro

Mark functions for benchmarking:

```rust
use mobench_sdk::benchmark;

#[benchmark]
fn my_expensive_operation() {
    let result = compute_something();
    std::hint::black_box(result);
}
```

### New CLI Commands

```bash
# Initialize SDK project
cargo mobench init-sdk --target android --project-name my-bench

# Build mobile artifacts
cargo mobench build --target android

# Package iOS app as IPA (for BrowserStack or physical devices)
cargo mobench package-ipa --method adhoc

# List discovered benchmarks
cargo mobench list
```

### Architecture

- **mobench-sdk**: Core library (registry, runner, builders, codegen)
- **mobench-macros**: `#[benchmark]` proc macro
- **mobench**: CLI tool for building, testing, and running benchmarks
- **examples/basic-benchmark**: Example using the new SDK

---

## Original README (Legacy Information)

## Layout

- `crates/mobench`: CLI orchestrator for building/packaging benchmarks and driving BrowserStack runs.
- `crates/bench-runner`: Shared harness that will be embedded in Android/iOS binaries; currently host-side only.
- `crates/sample-fns`: Small Rust functions used as demo benchmarks with UniFFI bindings for mobile platforms.
- `PROJECT_PLAN.md`: Goals, architecture outline, and initial task backlog.
- `android/`: Minimal Android app that loads the Rust demo library; Gradle project for BrowserStack/AppAutomate runs.

## Quick Start

### Host Demo (No Mobile Build Required)

Test the benchmarking harness locally:

```bash
cargo mobench demo --iterations 10 --warmup 2
```

### Mobile Testing

For complete end-to-end testing on Android/iOS, see the **[End-to-End Testing](#end-to-end-testing)** section below.

**Quick commands:**

- **Android**: `scripts/build-android-app.sh` then install APK
- **iOS**: `scripts/build-ios.sh` then open in Xcode

### Generate Config Files

```bash
cargo mobench init --output bench-config.toml
cargo mobench plan --output device-matrix.yaml
```

## UniFFI Bindings (Proc Macro Mode)

This project uses [UniFFI](https://mozilla.github.io/uniffi-rs/) with **proc macros** to generate type-safe Kotlin and Swift bindings from Rust code.

### Adding New FFI Types

No UDL file needed! Just add proc macro attributes to your Rust types:

```rust
#[derive(uniffi::Record)]
pub struct MyBenchmark {
    pub name: String,
    pub iterations: u32,
}

#[uniffi::export]
pub fn run_my_benchmark(spec: MyBenchmark) -> Result<BenchReport, BenchError> {
    // Your implementation
}

uniffi::setup_scaffolding!();  // Auto-uses crate name as namespace
```

### Regenerating Bindings

After modifying FFI types in `crates/sample-fns/src/lib.rs`:

```bash
# Build the library first
cargo build -p sample-fns

# Generate Kotlin + Swift bindings
./scripts/generate-bindings.sh
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

## Testing Workflows

mobile-bench-rs supports two testing workflows:

1. **[Local Development](#local-development-workflow)**: Test on emulators/simulators or connected devices using Android Studio/Xcode
2. **[BrowserStack Testing](#browserstack-workflow)**: Test on real devices in the cloud using BrowserStack App Automate

---

## Local Development Workflow

Test your benchmarks locally using Android Studio or Xcode. This is the fastest way to iterate during development.

### Android (Local)

#### Quick Start (All-in-One)

```bash
# Build everything and create APK
# Set UNIFFI_ANDROID_ABI for emulator ABI (x86_64 for default Android Studio emulators).
UNIFFI_ANDROID_ABI=x86_64 scripts/build-android-app.sh

# Install and launch on emulator/device
adb install -r android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity
```

#### Step-by-Step

```bash
# 1. Build Rust libraries + regenerate bindings (ABI-aware) + sync jniLibs
UNIFFI_ANDROID_ABI=x86_64 scripts/build-android-app.sh

# 2. Build the APK with Gradle
cd android && ./gradlew :app:assembleDebug

# 3. Install and launch
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

### iOS (Local)

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

### Key Features

The build process is **streamlined** with UniFFI proc macros:

- ✅ No UDL file needed - proc macros define the FFI from Rust code
- ✅ No need to run `cbindgen` manually
- ✅ UniFFI headers (`sample_fnsFFI.h`) are automatically generated during `build-ios.sh`
- ✅ Kotlin/Swift bindings are already committed to git
- ✅ Only regenerate bindings if you change FFI types in Rust (via `./scripts/generate-bindings.sh`)
- ✅ Apps show formatted output with statistics (min/max/avg in microseconds)
- ✅ Type-safe error handling (no more string parsing)

---

## BrowserStack Workflow

Test your benchmarks on real devices in the cloud using BrowserStack App Automate. This workflow uploads your app to BrowserStack, runs tests remotely, and downloads results.

### Prerequisites

1. **BrowserStack Account**: Sign up at [browserstack.com](https://www.browserstack.com/)
2. **Credentials**: Set environment variables:
   ```bash
   export BROWSERSTACK_USERNAME="your_username"
   export BROWSERSTACK_ACCESS_KEY="your_access_key"
   ```
3. **Built Artifacts**: Build your app and test suite first (see below)

### Android + BrowserStack (Espresso)

#### Step 1: Build Artifacts

```bash
# Build Android app APK and test suite
UNIFFI_ANDROID_ABI=x86_64 ./scripts/build-android-app.sh

# Build test APK (if needed)
cd android
./gradlew :app:assembleDebugAndroidTest
cd ..
```

Artifacts created:
- **App APK**: `android/app/build/outputs/apk/debug/app-debug.apk`
- **Test APK**: `android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`

#### Step 2: Run on BrowserStack

```bash
# Run benchmark on specific device
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --devices "Pixel 7-13" \
  --output run-summary.json
```

**What happens:**
1. CLI uploads APKs to BrowserStack
2. Schedules Espresso test run on specified device
3. Waits for completion
4. Downloads logs and results
5. Saves summary to `run-summary.json`

#### Step 3: View Results

```bash
# Results are in run-summary.json
cat run-summary.json

# BrowserStack artifacts downloaded to:
# target/browserstack/{build_id}/session-{session_id}/
```

**BrowserStack Dashboard**: View live test execution at https://app-automate.browserstack.com/dashboard

### iOS + BrowserStack (XCUITest)

#### Step 1: Build Artifacts

```bash
# Build iOS app and xcframework
./scripts/build-ios.sh

# Generate Xcode project
cd ios/BenchRunner
xcodegen generate

# Build app for device (requires signing)
xcodebuild -project BenchRunner.xcodeproj \
  -scheme BenchRunner \
  -sdk iphoneos \
  -configuration Release \
  -derivedDataPath build \
  CODE_SIGN_IDENTITY="-" \
  CODE_SIGNING_REQUIRED=NO \
  CODE_SIGNING_ALLOWED=NO

# Create IPA
mkdir -p Payload
cp -r build/Build/Products/Release-iphoneos/BenchRunner.app Payload/
zip -r BenchRunner.ipa Payload/
mv BenchRunner.ipa ../../target/ios/

# Build XCUITest runner
xcodebuild build-for-testing \
  -project BenchRunner.xcodeproj \
  -scheme BenchRunner \
  -sdk iphoneos \
  -derivedDataPath build

# Package test runner
cd build/Build/Products/Release-iphoneos
zip -r BenchRunnerUITests-Runner.zip BenchRunnerUITests-Runner.app
mv BenchRunnerUITests-Runner.zip ../../../../target/ios/
cd ../../../..
```

Artifacts created:
- **App IPA**: `target/ios/BenchRunner.ipa`
- **Test Suite**: `target/ios/BenchRunnerUITests-Runner.zip`

#### Step 2: Run on BrowserStack

```bash
# Run benchmark on specific device
cargo mobench run \
  --target ios \
  --function sample_fns::fibonacci \
  --iterations 100 \
  --warmup 10 \
  --devices "iPhone 14-16" \
  --ios-app target/ios/BenchRunner.ipa \
  --ios-test-suite target/ios/BenchRunnerUITests-Runner.zip \
  --output run-summary.json
```

**What happens:**
1. CLI uploads IPA and test suite to BrowserStack
2. Schedules XCUITest run on specified device
3. Waits for completion
4. Downloads logs and results
5. Saves summary to `run-summary.json`

### Using Config Files (Recommended)

For repeated runs, use config files:

```bash
# Generate templates
cargo mobench init --output bench-config.toml --target android
cargo mobench plan --output device-matrix.yaml
```

**bench-config.toml:**
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
```

**device-matrix.yaml:**
```yaml
devices:
  - name: Pixel 7
    os: android
    os_version: "13.0"
    tags: [default, pixel]
  - name: Samsung Galaxy S23
    os: android
    os_version: "13.0"
    tags: [samsung]
```

**Run with config:**
```bash
cargo mobench run --config bench-config.toml
```

### BrowserStack Features

- **Device Logs**: Automatically downloaded to `target/browserstack/{build_id}/session-{session_id}/`
- **Screenshots/Video**: Available in BrowserStack dashboard
- **Parallel Testing**: Specify multiple devices to run in parallel
- **Network Conditions**: Configure via BrowserStack dashboard
- **Real Devices**: Tests run on actual hardware, not emulators

---

## Requirements

### Android

- Android Studio (SDK + NDK manager): https://developer.android.com/studio
- Android NDK (API level 24+): https://developer.android.com/ndk/downloads
- `ANDROID_NDK_HOME` environment variable set
- `cargo-ndk` installed: `cargo install cargo-ndk` (https://github.com/bbqsrc/cargo-ndk)
- JDK 17+ (for Gradle; any distribution): https://openjdk.org/install/
  - Note: Android Gradle Plugin (AGP) officially supports Java 17.
- For local testing: Android emulator or physical device
- For BrowserStack: BrowserStack account and credentials

### iOS

- macOS with Xcode command-line tools: https://developer.apple.com/xcode/
- Rust targets: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim` (https://doc.rust-lang.org/rustup/targets.html)
- `xcodegen` installed (optional): https://github.com/yonaskolb/XcodeGen
- For local testing: iOS Simulator or physical device (requires code signing)
- For BrowserStack: BrowserStack account and credentials

---

## Additional Documentation

- **`BUILD.md`**: Complete build reference guide for Android and iOS (prerequisites, step-by-step instructions, troubleshooting)
- **`TESTING.md`**: Comprehensive testing guide with troubleshooting and advanced scenarios
- **`PROJECT_PLAN.md`**: Project goals, architecture, and task backlog
- **`CLAUDE.md`**: Developer guide for this codebase

---

## License

MIT OR Apache-2.0
