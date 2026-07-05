# Build Guide

Current release: **0.1.43**.

Use `cargo mobench build --target <android|ios|both>` for repository
development and CI. The CLI resolves the benchmark crate, builds native Rust
artifacts, generates or updates mobile runner projects, and writes outputs to
`target/mobench/` by default.

## Prerequisite Checks

```bash
cargo mobench check --target android
cargo mobench check --target ios
cargo mobench check --target both
```

Use JSON output for CI diagnostics:

```bash
cargo mobench check --target android --format json
```

## Android Prerequisites

- Android SDK
- Android NDK through `ANDROID_NDK_HOME` or SDK discovery
- `cargo-ndk`
- Rust targets:
  - `aarch64-linux-android`
  - `armv7-linux-androideabi`
  - `x86_64-linux-android`

Install targets:

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
```

Use `UNIFFI_ANDROID_ABI=x86_64` when testing the UniFFI Android path on a
default x86_64 emulator.

## iOS Prerequisites

- macOS
- Xcode command-line tools
- XcodeGen for generated project files
- Rust targets:
  - `aarch64-apple-ios`
  - `aarch64-apple-ios-sim`
  - `x86_64-apple-ios`

Install targets:

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
```

## Build Commands

Android:

```bash
cargo mobench build --target android --progress
```

iOS:

```bash
cargo mobench build --target ios --progress
```

Both:

```bash
cargo mobench build --target both --progress
```

Release builds:

```bash
cargo mobench build --target android --release
cargo mobench build --target ios --release
```

Custom output directory:

```bash
cargo mobench build \
  --target android \
  --output-dir target/custom-mobench
```

## Project Resolution

Build, run, list, verify, and package commands resolve the benchmark crate from:

1. `--project-root`
2. `--crate-path`
3. `mobench.toml`
4. Cargo workspace metadata
5. Git root
6. Legacy `bench-mobile/` layout

Use explicit flags in CI when ambiguity is possible:

```bash
cargo mobench build \
  --target android \
  --project-root . \
  --crate-path examples/basic-benchmark
```

## Generated Runner Backends

Configure the generated mobile runner backend in `mobench.toml`:

```toml
[project]
crate = "my-bench-crate"
library_name = "my_bench_crate"
ffi_backend = "uniffi" # or "native-c-abi"
```

`uniffi` is the compatibility default and generates Kotlin/Swift bindings.

`native-c-abi` calls the mobench JSON C ABI directly from generated Android and
iOS runners. Benchmark crates using this backend should export:

```rust
mobench_sdk::export_native_c_abi!();
```

## Android Outputs

Default Android outputs are written below `target/mobench/android/`.

Common artifacts:

- Generated Android project files
- `app/src/main/jniLibs/<abi>/lib<library_name>.so`
- APK files produced by Gradle
- Android test-suite APKs when test packaging is needed

The Android runner reads benchmark specs from intent extras or
`bench_spec.json` assets.

## iOS Outputs

Default iOS outputs are written below `target/mobench/ios/`.

The builder creates an xcframework similar to:

```text
target/mobench/ios/<library_name>.xcframework/
├── Info.plist
├── ios-arm64/
│   └── <library_name>.framework/
│       ├── <library_name>
│       ├── Headers/
│       └── Info.plist
└── ios-simulator-arm64/
    └── <library_name>.framework/
        ├── <library_name>
        ├── Headers/
        └── Info.plist
```

The framework binary and framework directory are named after the resolved
library name. Simulator slices use the `ios` platform with the `simulator`
variant in the xcframework manifest.

The build step attempts ad-hoc signing:

```bash
codesign --force --deep --sign - target/mobench/ios/<library_name>.xcframework
```

Package iOS BrowserStack artifacts:

```bash
cargo mobench package-ipa --method adhoc
cargo mobench package-xcuitest
```

## Verify Artifacts

```bash
cargo mobench verify \
  --target android \
  --check-artifacts \
  --output-dir target/mobench
```

Smoke-test one function through the host harness:

```bash
cargo mobench verify \
  --target android \
  --function sample_fns::fibonacci \
  --smoke-test
```

## Common Issues

- BrowserStack upload timeout: build with `--release`.
- Missing Android target: run `rustup target add <target>`.
- Missing NDK tools: set `ANDROID_NDK_HOME` or install the NDK through Android
  Studio.
- Unsigned iOS framework: rerun `cargo mobench build --target ios` or sign the
  generated xcframework with `codesign --force --deep --sign -`.
- Wrong iOS framework name: verify `library_name` in `mobench.toml`.
- Swift cannot find C types: verify the generated bridging header and C header
  are present in the generated iOS project.
