# Testing Guide

Current release: **0.1.49**.

This guide covers host tests, CLI validation, generated mobile artifacts,
BrowserStack smoke tests, and local native profiling checks.

## Host Tests

Run the full Rust test suite:

```bash
cargo test --all
```

Run a package-specific suite:

```bash
cargo test -p mobench-sdk
cargo test -p mobench-macros
cargo test -p mobench
```

## CLI Validation

Check local prerequisites:

```bash
cargo mobench check --target android
cargo mobench check --target ios
```

Validate BrowserStack credentials, config, and a device matrix:

```bash
cargo mobench doctor \
  --target both \
  --config bench-config.toml \
  --device-matrix device-matrix.yaml \
  --browserstack true
```

List registered benchmarks:

```bash
cargo mobench list --crate-path examples/basic-benchmark
```

Verify registry and generated artifacts:

```bash
cargo mobench verify \
  --target android \
  --crate-path examples/basic-benchmark \
  --check-artifacts
```

Run a host smoke test:

```bash
cargo mobench verify \
  --target android \
  --crate-path examples/basic-benchmark \
  --function basic_benchmark::bench_fibonacci \
  --smoke-test
```

## Local-Only Benchmark Runs

Use `--local-only` to exercise the host harness without mobile builds:

```bash
cargo mobench run \
  --target android \
  --function basic_benchmark::bench_fibonacci \
  --crate-path examples/basic-benchmark \
  --local-only \
  --iterations 20 \
  --warmup 5 \
  --output target/mobench/results.json
```

Inspect the report:

```bash
cargo mobench summary target/mobench/results.json
cargo mobench summary --format json target/mobench/results.json
cargo mobench summary --format csv target/mobench/results.json
```

## Android Testing

Build Android artifacts:

```bash
cargo mobench build --target android --progress
```

For default Android emulators using the UniFFI path:

```bash
UNIFFI_ANDROID_ABI=x86_64 cargo mobench build --target android --progress
```

Verify outputs:

```bash
cargo mobench verify \
  --target android \
  --check-artifacts \
  --output-dir target/mobench
```

Use `--release` for BrowserStack-sized artifacts:

```bash
cargo mobench build --target android --release
```

## iOS Testing

Build iOS artifacts:

```bash
cargo mobench build --target ios --progress
```

Verify outputs:

```bash
cargo mobench verify \
  --target ios \
  --check-artifacts \
  --output-dir target/mobench
```

Package BrowserStack iOS artifacts:

```bash
cargo mobench package-ipa --method adhoc
cargo mobench package-xcuitest
```

## BrowserStack Smoke Tests

Set credentials:

```bash
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"
```

Android:

```bash
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --iterations 20 \
  --warmup 5 \
  --release \
  --fetch \
  --output target/mobench/results.json
```

iOS:

```bash
cargo mobench run \
  --target ios \
  --function sample_fns::fibonacci \
  --devices "iPhone 14-16" \
  --iterations 20 \
  --warmup 5 \
  --release \
  --fetch \
  --output target/mobench/results.json
```

## CI Contract Test

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --local-only \
  --iterations 20 \
  --warmup 5 \
  --plots auto \
  --output-dir target/mobench/ci
```

Expected files:

- `target/mobench/ci/summary.json`
- `target/mobench/ci/summary.md`
- `target/mobench/ci/results.csv`
- `target/mobench/ci/plots/*.svg` when plots are rendered

## Profiling Checks

Android native profiling:

```bash
cargo mobench profile run \
  --target android \
  --provider local \
  --backend android-native \
  --function sample_fns::fibonacci
```

iOS simulator-host profiling:

```bash
cargo mobench profile run \
  --target ios \
  --provider local \
  --backend ios-instruments \
  --function sample_fns::fibonacci
```

Render the latest profile summary:

```bash
cargo mobench profile summarize \
  --profile target/mobench/profile/profile.json
```

BrowserStack native stack/flamegraph profiling is unsupported in this release.

## Troubleshooting

- No benchmarks listed: confirm functions use `#[benchmark]`, are public, and
  the crate depends on `inventory`.
- Android emulator build fails: use `UNIFFI_ANDROID_ABI=x86_64` for the default
  emulator ABI.
- BrowserStack upload is slow or times out: use `--release`.
- Missing BrowserStack artifacts: rerun with `--fetch` or use `cargo mobench
  fetch`.
- iOS framework is rejected as unsigned: sign the generated xcframework or rerun
  the iOS build.
