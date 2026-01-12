# bench-sdk Integration Guide

This guide shows how to integrate `bench-sdk` into an existing Rust project, run local
mobile benchmarks, and then run them on BrowserStack.

## 1) Add bench-sdk to your crate

In your project's `Cargo.toml`:

```toml
[dependencies]
bench-sdk = "0.1"
```

## 2) Annotate benchmark functions

Add `#[bench_sdk::benchmark]` to any function you want to run on devices.

```rust
use bench_sdk::benchmark;

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

## 3) Scaffold mobile projects

From your repo root, create a mobile harness with the CLI:

```bash
cargo run -p bench-cli -- init-sdk --target both --project-name my-bench --output-dir .
```

This generates:
- `bench-mobile/` (FFI bridge that links your crate)
- `android/` and `ios/` app templates
- `bench-sdk.toml` configuration

## 4) Local Android testing

Build the Android app with ABI-aware bindings (emulator uses x86_64):

```bash
UNIFFI_ANDROID_ABI=x86_64 ./scripts/build-android-app.sh
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

## 5) Local iOS testing

```bash
./scripts/build-ios.sh
open ios/BenchRunner/BenchRunner.xcodeproj
```

Run the app in Xcode. It will read `bench_spec.json` from the bundle or use defaults.

## 6) BrowserStack (Android Espresso)

Build APK + test APK:

```bash
UNIFFI_ANDROID_ABI=x86_64 ./scripts/build-android-app.sh
cd android
./gradlew :app:assembleDebugAndroidTest
cd ..
```

Run via CLI:

```bash
cargo run -p bench-cli -- run \
  --target android \
  --function my_crate::checksum_bench \
  --iterations 100 \
  --warmup 10 \
  --devices "Pixel 7-13.0"
```

## 7) BrowserStack (iOS XCUITest)

Build iOS artifacts, then provide the app and test suite:

```bash
./scripts/build-ios.sh

cargo run -p bench-cli -- run \
  --target ios \
  --function my_crate::checksum_bench \
  --iterations 100 \
  --warmup 10 \
  --devices "iPhone 14-16" \
  --ios-app path/to/BenchRunner.ipa \
  --ios-test-suite path/to/BenchRunnerUITests.zip
```

## Notes

- If you change FFI types, regenerate bindings: `./scripts/generate-bindings.sh`
- Android emulator ABI is typically `x86_64` in Android Studio.
- BrowserStack credentials must be set via `BROWSERSTACK_USERNAME` and `BROWSERSTACK_ACCESS_KEY`.
