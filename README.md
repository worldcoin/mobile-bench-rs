# mobile-bench-rs
Benchmarking tool for Rust functions on mobile devices using BrowserStack.

## Layout
- `crates/bench-cli`: CLI orchestrator for building/packaging benchmarks and driving BrowserStack runs (stubbed).
- `crates/bench-runner`: Shared harness that will be embedded in Android/iOS binaries; currently host-side only.
- `crates/sample-fns`: Small Rust functions used as demo benchmarks with UniFFI bindings for mobile platforms.
- `PROJECT_PLAN.md`: Goals, architecture outline, and initial task backlog.
- `android/`: Minimal Android app that loads the Rust demo library; Gradle project for BrowserStack/AppAutomate runs.

## Quick start (host demo)
The mobile pieces are not wired up yet, but you can exercise the host-side harness:

```bash
cargo run -p bench-cli -- demo --iterations 10 --warmup 2
```

Generate starter config files (will refuse to overwrite existing files):
```bash
cargo run -p bench-cli -- init --output bench-config.toml
cargo run -p bench-cli -- plan --output device-matrix.yaml
```

Build the Android demo APK (requires Android SDK/NDK/Gradle):
```bash
scripts/build-android.sh
scripts/sync-android-libs.sh
cd android && gradle :app:assembleDebug
```

To exercise the Android app with different parameters, pass intent extras when launching:
```bash
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function sample_fns::checksum \
  --ei bench_iterations 30 \
  --ei bench_warmup 5
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

## iOS harness (SwiftUI)
- Build the Rust xcframework (uses UniFFI-generated headers):
  ```bash
  scripts/build-ios.sh
  ```
  Outputs land under `target/ios/sample_fns.xcframework/`.
- Generate the Xcode project with XcodeGen:
  ```bash
  cd ios/BenchRunner
  xcodegen generate
  ```
  The project expects `sample_fns.xcframework` at `../../target/ios/sample_fns.xcframework`. Adjust the path in `project.yml` if your build tool writes elsewhere.
- Run the BenchRunner app; it reads params from environment or launch args:
  - `BENCH_FUNCTION` / `--bench-function=sample_fns::checksum`
  - `BENCH_ITERATIONS` / `--bench-iterations=30`
  - `BENCH_WARMUP` / `--bench-warmup=5`
  The app uses UniFFI-generated Swift bindings to call `runBenchmark(spec:)` and displays formatted results with timing statistics.

Refer to `PROJECT_PLAN.md` for the roadmap and next steps.

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
