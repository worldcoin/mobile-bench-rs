# BrowserStack CI Guide

Current release: **0.2.0**.

This guide covers BrowserStack benchmark execution, deterministic device
resolution, CI contract outputs, artifact fetching, PR reporting, and baseline
regression checks. Native stack/flamegraph profiling remains local-only.

## Credentials

Credentials resolve from config, environment variables, then `.env.local`:

```bash
export BROWSERSTACK_USERNAME="your_username"
export BROWSERSTACK_ACCESS_KEY="your_access_key"
export BROWSERSTACK_PROJECT="mobile-benchmarks"
```

Config form:

```toml
[browserstack]
app_automate_username = "${BROWSERSTACK_USERNAME}"
app_automate_access_key = "${BROWSERSTACK_ACCESS_KEY}"
project = "mobile-benchmarks"
```

Validate setup:

```bash
cargo mobench doctor \
  --target both \
  --config bench-config.toml \
  --device-matrix device-matrix.yaml \
  --browserstack true
```

## Device Resolution

List and validate BrowserStack devices:

```bash
cargo mobench devices --platform android
cargo mobench devices --platform ios
cargo mobench devices --json
cargo mobench devices --validate "Google Pixel 7-13.0"
```

Resolve a device matrix/profile deterministically:

```bash
cargo mobench devices resolve \
  --platform android \
  --profile default \
  --device-matrix device-matrix.yaml
```

Example `device-matrix.yaml`:

```yaml
devices:
- name: "Google Pixel 7-13.0"
os: "android"
os_version: "13.0"
tags: ["default", "pixel"]
- name: "iPhone 14-16"
os: "ios"
os_version: "16"
tags: ["default", "iphone"]
```

Invalid device specs return suggestions:

```text
Invalid devices (1):
  [ERROR] Google Pixle 7-13.0: Device not found
          Suggestions:
            - Google Pixel 7-13.0
            - Google Pixel 7 Pro-13.0
```

### Legacy iOS Devices

mobench supports two generated iOS runner templates:

- `swiftui`: default iOS 15+ runner for current CI lanes.
- `uikit-legacy`: UIKit runner without SwiftUI or Swift concurrency for lower deployment targets.

Older BrowserStack iOS devices require both the legacy runner and an older
Xcode toolchain. BrowserStack App Automate currently lists iPhone 7 as iOS 10,
so an iPhone 7 legacy lane needs Xcode capable of building and installing for
iOS 10/11/12.

```toml
[ios]
deployment_target = "10.0"
runner = "uikit-legacy"
```

```yaml
devices:
- name: "iPhone 7-10"
os: "ios"
tags: ["legacy-ios"]
```

mobench fails early if the selected BrowserStack iOS device version is lower
than the app deployment target, for example `iPhone 7-10` with
`deployment_target = "15.0"`. It also checks the selected local Xcode version
before iOS builds and reports when a legacy deployment target needs an older
Xcode lane.

### Verify Benchmark Setup

```bash
cargo mobench verify --target android --check-artifacts
cargo mobench verify --target android --check-artifacts --function my_benchmark
cargo mobench report summarize --summary target/mobench/ci/summary.json
```

`cargo mobench verify --smoke-test` is only supported for benchmark crates
linked into the `mobench` CLI binary. External crates discovered through
`mobench.toml`, `--project-root`, or `--crate-path` should use
`cargo mobench list` and `cargo mobench verify --check-artifacts`.

## One-Command CI Run

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --plots auto \
  --output-dir target/mobench/ci
```

Default outputs:

- `target/mobench/ci/summary.json`
- `target/mobench/ci/summary.md`
- `target/mobench/ci/results.csv`
- `target/mobench/ci/plots/*.svg` when plot rendering is available

`results.csv` timing columns include:

- `device`
- `function`
- `samples`
- `mean_ns`
- `median_ns`
- `p95_ns`
- `min_ns`
- `max_ns`

Resource columns include:

- `cpu_total_ms`
- `cpu_median_ms`
- `peak_memory_kb`
- `peak_memory_growth_kb`
- `process_peak_memory_kb`

Missing resource data is emitted as blank CSV fields.

## Split-Sample Merge

Long or fragile BrowserStack lanes can run each measured sample as its own CI
invocation. Put one-sample outputs under directories like
`sample-1/summary.json`, `sample-2/summary.json`, then merge them back into the
standard CI output contract:

```bash
cargo mobench ci merge-split-runs \
--samples-dir target/mobench/ci/android/sample_fns__fibonacci/device/split \
--output-dir target/mobench/ci/android/sample_fns__fibonacci/device \
--function sample_fns::fibonacci \
--device "Google Pixel 7-13.0" \
--iterations 5 \
--warmup 1
```

The command validates that every input summary contains exactly one device and
one benchmark, that the benchmark matches `--function`, that every sample is for
`--device`, and that the merged measured sample count equals `--iterations`.
It writes `summary.json`, `summary.md`, and `results.csv` in `--output-dir`, so
normal report, plot, PR comment, and comparison tooling can consume merged
output unchanged.

## Multiple Functions

```bash
cargo mobench ci run \
  --target android \
  --functions '["sample_fns::fibonacci","sample_fns::checksum"]' \
  --devices "Google Pixel 7-13.0" \
  --release \
  --output-dir target/mobench/ci
```

## GitHub Action Usage

Minimal workflow:

```yaml
name: Mobench CI

on:
  workflow_dispatch:

permissions:
  contents: read
  pull-requests: write

jobs:
  mobench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: aarch64-linux-android
      - uses: ./.github/actions/mobench
        with:
          command: cargo mobench ci run
          run-args: |
            --target android
            --function sample_fns::fibonacci
            --iterations 20
            --warmup 5
            --local-only
            --plots auto
          pr-comment: true
          github-token: ${{ github.token }}
```

Use `--release` for BrowserStack runs. Debug Android artifacts can be large and
may time out during upload.

## PR Reporting

Render Markdown from an existing summary:

```bash
cargo mobench report summarize \
  --summary target/mobench/ci/summary.json \
  --plots auto
```

Generate or publish a sticky PR comment:

```bash
cargo mobench report github \
  --pr 123 \
  --summary target/mobench/ci/summary.json \
  --publish
```

The default marker is `<!-- mobench-report -->`.

## Baselines And Regressions

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --baseline artifact:target/mobench/ci/summary.json \
  --regression-threshold-pct 5 \
  --junit target/mobench/ci/junit.xml \
  --output-dir target/mobench/ci
```

Regression comparisons use summary JSON. If the baseline path resolves to the
same path as the candidate output, mobench snapshots the prior file before
writing the candidate summary.

## Fetching BrowserStack Artifacts

Fetch during `run` or `ci run`:

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --fetch-output-dir target/browserstack
```

Fetch later:

```bash
cargo mobench fetch \
  --target android \
  --build-id <browserstack-build-id> \
  --output-dir target/browserstack \
  --wait
```

Fetched artifacts include session JSON plus available log/video URLs from
BrowserStack. Downloads are restricted to BrowserStack HTTPS hosts.

## Limits

- BrowserStack benchmark runs can provide timing and resource metrics.
- BrowserStack native stack/flamegraph capture is unsupported.
- Use `cargo mobench profile run --provider local` for native profiling.
