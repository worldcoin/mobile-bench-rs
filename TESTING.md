# Testing Guide

This document provides comprehensive testing instructions for mobile-bench-rs.

> **For SDK Integrators**: If you're importing `mobench-sdk` into your project, use:
> - `cargo mobench build --target <android|ios>` for builds
> - Scripts shown below are legacy tooling for this repository
> - See [BENCH_SDK_INTEGRATION.md](BENCH_SDK_INTEGRATION.md) for the integration guide
> **Note**: For detailed build instructions, prerequisites, and step-by-step build processes, see **[BUILD.md](BUILD.md)**. This document focuses on testing scenarios and troubleshooting.

## Table of Contents
- [Prerequisites](#prerequisites)
- [Host Testing](#host-testing)
- [Android Testing](#android-testing)
- [iOS Testing](#ios-testing)
- [Troubleshooting](#troubleshooting)

## Prerequisites

### Rust
```bash
# Install Rust if not already installed
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# https://www.rust-lang.org/tools/install

# Install required targets
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
# https://doc.rust-lang.org/rustup/targets.html

# Install cargo-ndk for Android builds
cargo install cargo-ndk
# https://github.com/bbqsrc/cargo-ndk
```

### Android
```bash
# Install Android SDK and NDK (via Android Studio or command line)
# Android Studio: https://developer.android.com/studio
# Android NDK: https://developer.android.com/ndk/downloads
# JDK 17+ (for Gradle; any distribution): https://openjdk.org/install/
# Note: Android Gradle Plugin (AGP) officially supports Java 17.
# Set environment variable (add to ~/.zshrc or ~/.bashrc)
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/29.0.14206865

# Verify NDK is available
ls $ANDROID_NDK_HOME
```

### iOS (macOS only)
```bash
# Install Xcode from App Store
# https://developer.apple.com/xcode/
# Install command-line tools
xcode-select --install

# Install xcodegen
brew install xcodegen
# https://github.com/yonaskolb/XcodeGen
```

## Host Testing

### Unit Tests
Run all Rust tests:
```bash
cargo test --all
```

Expected output: All tests pass (11 tests total as of UniFFI migration).

### CLI Note
The CLI does not currently expose a host-only demo command. Use `cargo test --all` for host
validation and use `cargo mobench run` to execute benchmarks on devices.

### CI Artifacts
The `Mobile Bench (manual)` workflow uploads summary artifacts:
- `host-run-summary` (JSON + Markdown + optional CSV from host-only run)
- `browserstack-run-summary` (JSON + Markdown + optional CSV + fetched logs when secrets are set)

## Android Testing

### Method 1: Quick All-in-One Build

```bash
# Build everything and create APK
cargo mobench build --target android

# Install on connected device/emulator
adb install -r target/mobench/android/app/build/outputs/apk/debug/app-debug.apk

# Launch app
adb shell am start -n dev.world.bench/.MainActivity
```

### Method 2: Step-by-Step Build

```bash
# Step 1: Build Rust libraries + bindings (ABI-aware)
cargo mobench build --target android

# Step 2: Build APK
cd target/mobench/android
./gradlew :app:assembleDebug
cd ../../..

# Step 3: Install and launch
adb install -r target/mobench/android/app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n dev.world.bench/.MainActivity
```

### Method 3: Using Android Studio

1. Build Rust libraries first:
   ```bash
   cargo mobench build --target android
   ```

2. Open `target/mobench/android/` directory in Android Studio

3. Let Gradle sync complete

4. Click Run (green triangle) or Run → Run 'app'

5. Select target device/emulator

### Testing with Custom Parameters

Launch with different benchmark configurations:

```bash
# Test checksum function with 30 iterations
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function sample_fns::checksum \
  --ei bench_iterations 30 \
  --ei bench_warmup 5

# Test fibonacci with minimal runs
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function fibonacci \
  --ei bench_iterations 5 \
  --ei bench_warmup 1
```

Parameters:
- `--es bench_function <string>`: Function name (fibonacci, checksum, sample_fns::fibonacci, etc.)
- `--ei bench_iterations <int>`: Number of benchmark iterations
- `--ei bench_warmup <int>`: Number of warmup iterations

### Verifying Output

Check logcat for detailed output:
```bash
adb logcat | grep -i bench
```

The app display should show:
```
=== Benchmark Results ===

Function: sample_fns::fibonacci
Iterations: 20
Warmup: 3

Samples (20):
  1. 0.001 ms
  2. 0.001 ms
  ...

Statistics:
  Min: 0.001 ms
  Max: 0.002 ms
  Avg: 0.001 ms
```

## iOS Testing

### Build and Run

```bash
# Step 1: Build Rust xcframework (includes automatic code signing)
cargo mobench build --target ios

# This script:
# - Compiles Rust for aarch64-apple-ios (device) and aarch64-apple-ios-sim (simulator)
# - Creates xcframework with proper structure:
#   target/mobench/ios/sample_fns.xcframework/
#     ├── Info.plist
#     ├── ios-arm64/
#     │   └── sample_fns.framework/
#     │       ├── sample_fns (binary)
#     │       ├── Headers/
#     │       │   ├── sample_fnsFFI.h
#     │       │   └── module.modulemap
#     │       └── Info.plist
#     └── ios-simulator-arm64/
#         └── sample_fns.framework/
#             ├── sample_fns (binary)
#             ├── Headers/
#             │   ├── sample_fnsFFI.h
#             │   └── module.modulemap
#             └── Info.plist
# - Copies UniFFI-generated C headers into framework
# - Creates module map for Swift to import C FFI
# - Automatically code-signs the xcframework

# Step 2: Generate Xcode project from project.yml
cd target/mobench/ios/BenchRunner
xcodegen generate

# Step 3: Open in Xcode
open BenchRunner.xcodeproj
```

In Xcode:
1. Select a scheme: BenchRunner
2. Select a destination: iPhone 15 (or any simulator)
3. Click Run (⌘R) or Product → Run

**Important Notes:**
- The xcframework contains static libraries (`.a` archives), not dynamic frameworks
- A bridging header (`BenchRunner-Bridging-Header.h`) is used to expose C FFI types to Swift
- The UniFFI-generated Swift bindings (`sample_fns.swift`) are compiled directly into the app
- No `import sample_fns` is needed - the types are available globally via the bridging header

### Testing with Custom Parameters

#### Method 1: Edit Scheme in Xcode

1. Product → Scheme → Edit Scheme...
2. Click "Run" in left sidebar
3. Go to "Arguments" tab
4. Click "Environment Variables" section
5. Click "+" to add variables:
   - Name: `BENCH_FUNCTION`, Value: `sample_fns::checksum`
   - Name: `BENCH_ITERATIONS`, Value: `30`
   - Name: `BENCH_WARMUP`, Value: `5`
6. Click Close
7. Run the app

#### Method 2: Command Line (Simulator Only)

First, build and install to simulator:
```bash
# Build for simulator
xcodebuild -project target/mobench/ios/BenchRunner/BenchRunner.xcodeproj \
  -scheme BenchRunner \
  -destination 'platform=iOS Simulator,name=iPhone 15' \
  -derivedDataPath target/mobench/ios/build

# Launch with arguments
xcrun simctl launch booted dev.world.bench.BenchRunner \
  --bench-function=sample_fns::checksum \
  --bench-iterations=30 \
  --bench-warmup=5
```

#### Method 3: Edit bench_spec.json Bundle Resource

Add `bench_spec.json` to the app bundle:
1. Create `target/mobench/ios/BenchRunner/BenchRunner/Resources/bench_spec.json`:
   ```json
   {
     "function": "sample_fns::checksum",
     "iterations": 30,
     "warmup": 5
   }
   ```
2. Add to Xcode project (File → Add Files to "BenchRunner"...)
3. Ensure it's in "Copy Bundle Resources" build phase
4. Run the app

### Verifying Output

The app should display:
```
=== Benchmark Results ===

Function: sample_fns::fibonacci
Iterations: 20
Warmup: 3

Samples (20):
  1. 0.001 ms
  2. 0.001 ms
  ...

Statistics:
  Min: 0.001 ms
  Max: 0.002 ms
  Avg: 0.001 ms
```

## Troubleshooting

### Android

**Problem**: `ANDROID_NDK_HOME is not set`
```bash
# Solution: Export the NDK path
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/29.0.14206865
# Or add to ~/.zshrc / ~/.bashrc
```

**Problem**: `cargo-ndk: command not found`
```bash
# Solution: Install cargo-ndk
cargo install cargo-ndk
```

**Problem**: `error: failed to run custom build command for 'sample-fns'`
```bash
# Solution: Clean and rebuild
cargo clean
cargo mobench build --target android
```

**Problem**: App crashes on launch with "UnsatisfiedLinkError"
```bash
# Solution: Ensure .so files are in the APK
cargo mobench build --target android
cd target/mobench/android && ./gradlew clean assembleDebug
```

**Problem**: App shows "Error: UnknownFunction"
- Check function name matches one of: `fibonacci`, `fib`, `sample_fns::fibonacci`, `checksum`, `checksum_1k`, `sample_fns::checksum`
- Function names are case-sensitive

### iOS

**Problem**: `xcodegen: command not found`
```bash
# Solution: Install xcodegen
brew install xcodegen
```

**Problem**: "The Framework 'sample_fns.xcframework' is unsigned"
```bash
# Solution: Code-sign the xcframework
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework

# The build step includes signing, but if you built manually:
cargo mobench build --target ios
cd target/mobench/ios/BenchRunner
xcodegen generate
# Clean build in Xcode (⌘+Shift+K) then build (⌘+B)
```

**Problem**: "No such module 'sample_fns'" or "Unable to find module dependency: 'sample_fns'" in Swift
```bash
# Solution: The Swift bindings are compiled directly into the app.
# Remove any `import sample_fns` statements from your Swift code.
# The types (BenchSpec, BenchReport, etc.) are available without import.
```

**Problem**: "Cannot find type 'RustBuffer' in scope" or FFI type errors
```bash
# Solution: Ensure the bridging header is configured
# Check that BenchRunner-Bridging-Header.h exists at:
# target/mobench/ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h

# If missing, create it with:
cat > target/mobench/ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h << 'EOF'
//
//  BenchRunner-Bridging-Header.h
//  BenchRunner
//
//  Bridge to import C FFI from Rust (UniFFI-generated)
//

#import "sample_fnsFFI.h"
EOF

# Then regenerate the Xcode project:
cd target/mobench/ios/BenchRunner
xcodegen generate
```

**Problem**: Build fails with "library not found for -lsample_fns" or "framework 'ios-simulator-arm64' not found"
```bash
# Solution: Ensure xcframework was built correctly with proper structure
rm -rf target/mobench/ios/sample_fns.xcframework
cargo mobench build --target ios
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework

# Verify structure:
ls -la target/mobench/ios/sample_fns.xcframework/
# Should show:
#   ios-arm64/sample_fns.framework/
#   ios-simulator-arm64/sample_fns.framework/
#   Info.plist
```

**Problem**: "While building for iOS Simulator, no library for this platform was found"
```bash
# Solution: Rebuild the xcframework - the structure may be incorrect
rm -rf target/mobench/ios/sample_fns.xcframework
cargo mobench build --target ios
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework

# Clean Xcode build folder
cd target/mobench/ios/BenchRunner
xcodebuild clean -project BenchRunner.xcodeproj -scheme BenchRunner
# Then build in Xcode
```

**Problem**: "Framework had an invalid CFBundleIdentifier in its Info.plist"
```bash
# Solution: The framework bundle ID should not conflict with the app
# Check the iOS builder uses `dev.world.sample-fns` for the framework
# Rebuild:
cargo mobench build --target ios
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework
```

**Problem**: Simulator crashes with "Symbol not found"
```bash
# Solution: Clean and rebuild for simulator architecture
cargo clean
cargo mobench build --target ios
codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework

# In Xcode, clean (⌘+Shift+K) then build (⌘+B)
```

**Problem**: "Could not launch" on physical device
- Ensure proper code signing is configured in Xcode
- Select your development team in Xcode → Project Settings → Signing & Capabilities
- Trust developer certificate on device: Settings → General → VPN & Device Management
- The xcframework must be signed: `codesign --force --deep --sign - target/mobench/ios/sample_fns.xcframework`

### UniFFI Bindings (Proc Macros)

**Problem**: Changes to FFI types in `crates/sample-fns/src/lib.rs` not reflected in mobile apps
```bash
# Solution: Rebuild library and regenerate bindings
cargo mobench build --target android

# Then rebuild mobile apps
cargo mobench build --target android
cargo mobench build --target ios
```

**Problem**: "error: cannot find type `BenchSpec` in the crate root"
```bash
# Solution: Ensure build.rs runs and generates scaffolding
cargo clean
cargo build -p sample-fns
```

### General

**Problem**: Tests fail after code changes
```bash
# Solution: Run tests to see specific failures
cargo test --all

# Common causes:
# - Missing serde dependency (check Cargo.toml)
# - API signature changes (update FFI types with proc macros and regenerate bindings)
# - Test assertions need updating
```

## Advanced Testing

### BrowserStack Integration Testing

See the main [README.md](README.md) for BrowserStack testing instructions.

### Performance Regression Testing

Compare benchmark results across builds:
```bash
# Run benchmark and save results
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --iterations 100 \
  --fetch \
  --output results-v1.json \
  --summary-csv

# After changes, run again
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --iterations 100 \
  --fetch \
  --output results-v2.json \
  --summary-csv

# Compare summaries
cargo mobench compare \
  --baseline results-v1.json \
  --candidate results-v2.json \
  --output comparison.md
```

### Adding New Test Functions

1. Add function to `crates/sample-fns/src/lib.rs`
2. Add to dispatch in `run_benchmark()` match statement
3. Add test case in `#[cfg(test)]` module
4. Run tests: `cargo test -p sample-fns`
5. Test on mobile platforms

Example:
```rust
// In lib.rs
pub fn my_new_function(n: u32) -> u64 {
    // implementation
}

// In run_benchmark()
"my_new_function" | "sample_fns::my_new_function" => {
    run_closure(runner_spec, || {
        let _ = my_new_function(100);
        Ok(())
    })
    .map_err(|e: BenchRunnerError| -> BenchError { e.into() })?
}

// In tests
#[test]
fn test_my_new_function() {
    let spec = BenchSpec {
        name: "my_new_function".to_string(),
        iterations: 3,
        warmup: 1,
    };
    let report = run_benchmark(spec).unwrap();
    assert_eq!(report.samples.len(), 3);
}
```

## Continuous Integration

The project includes a GitHub Actions workflow (`.github/workflows/mobile-bench.yml`) that:
- Runs host tests on every push
- Builds Android APK (optional)
- Builds iOS xcframework (optional)
- Uploads artifacts

To trigger manually:
1. Go to GitHub Actions tab
2. Select "mobile-bench-rs CI"
3. Click "Run workflow"
4. Select platform(s) to build

## Additional Resources

- [UniFFI Documentation](https://mozilla.github.io/uniffi-rs/)
- [Android NDK Documentation](https://developer.android.com/ndk)
- [Rust Cross-Compilation Guide](https://rust-lang.github.io/rustup/cross-compilation.html)
- [PROJECT_PLAN.md](PROJECT_PLAN.md) - Roadmap and architecture
- [CLAUDE.md](CLAUDE.md) - Developer guide for this codebase
