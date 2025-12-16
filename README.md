# mobile-bench-rs
Benchmarking tool for Rust functions on mobile devices using BrowserStack.

## Layout
- `crates/bench-cli`: CLI orchestrator for building/packaging benchmarks and driving BrowserStack runs (stubbed).
- `crates/bench-runner`: Shared harness that will be embedded in Android/iOS binaries; currently host-side only.
- `crates/sample-fns`: Small Rust functions used as demo benchmarks and FFI entrypoints.
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

Generate a C header for the shared library with cbindgen (install via `cargo install cbindgen`):
```bash
cbindgen crates/sample-fns --config crates/sample-fns/cbindgen.toml --output target/sample_fns.h
```
Key C ABI symbols:
- `bench_run_json(const char* function, uint32_t iterations, uint32_t warmup)` → heap C string with JSON report or error text.
- `bench_free_string(char* ptr)` to free strings returned by `bench_run_json`.
- `bench_fib_24()` and `bench_checksum_1k()` for simple fixed benchmarks.

## iOS harness (SwiftUI)
- Build the Rust xcframework + header:
  ```bash
  scripts/build-ios.sh
  ```
  Outputs land under `target/ios/` (including `sample_fns.h` in `target/ios/include`).
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
  The app calls `bench_run_json` and displays the JSON report in a monospaced text view.

Refer to `PROJECT_PLAN.md` for the roadmap and next steps.
