# mobench-sdk Integration Guide

This guide shows how to integrate `mobench-sdk` into an existing Rust project, run local
mobile benchmarks, and then run them on BrowserStack.

> **Important**: This guide is for integrators importing `mobench-sdk` as a library.
> You do **NOT** need the `scripts/` directory from this repository.
> All build functionality is available via `cargo mobench` commands.

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
fn checksum_bench() {
    let data = [1u8; 1024];
    let sum: u64 = data.iter().map(|b| *b as u64).sum();
    std::hint::black_box(sum);
}
```

Benchmarks are identified by name at runtime. You can call them by:
- Fully-qualified path (e.g., `my_crate::checksum_bench`)
- Or suffix match (e.g., `checksum_bench`)

## 4) Scaffold mobile projects

From your repo root, create a mobile harness with the CLI:

```bash
cargo mobench init-sdk --target both --project-name my-bench --output-dir .
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
  --devices "Google Pixel 7-13.0"
```

The CLI will automatically:
- Upload APK and test APK to BrowserStack
- Schedule the test run
- Wait for completion
- Download results and logs

## 8) BrowserStack (iOS XCUITest)

Build iOS artifacts and package as IPA:

```bash
# Build xcframework
cargo mobench build --target ios

# Package as IPA (ad-hoc signing, no Apple ID needed)
cargo mobench package-ipa --method adhoc

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
  --ios-app target/ios/BenchRunner.ipa \
  --ios-test-suite target/ios/BenchRunnerUITests.zip
```

**IPA Signing Methods:**
- `adhoc`: No Apple ID required, works for BrowserStack device testing
- `development`: Requires Apple Developer account, for physical device testing

## Notes

- **No scripts needed**: All functionality is available via `cargo mobench` commands
- If you change FFI types, the build process automatically regenerates bindings
- Android emulator ABI is typically `x86_64` in Android Studio
- BrowserStack credentials must be set via `BROWSERSTACK_USERNAME` and `BROWSERSTACK_ACCESS_KEY`
- For developing this repo (not integrating the SDK), legacy `scripts/` are available but deprecated
