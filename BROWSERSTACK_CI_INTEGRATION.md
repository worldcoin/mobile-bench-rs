# BrowserStack CI Integration Guide

This guide shows how to run benchmarks on BrowserStack and fetch results within CI pipelines.

## Overview

The BrowserStack client now supports:
1. **Prerequisite checking** - Verify tools and credentials before builds
2. **Device validation** - Validate device specs before scheduling runs
3. **Scheduling runs** - Upload artifacts and start tests
4. **Polling for completion** - Wait for tests to finish (with timeout)
5. **Fetching results** - Download device logs and extract benchmark data
6. **Result analysis** - Display summary statistics from reports

## Pre-flight Validation

Before running benchmarks on BrowserStack, validate your setup:

### Check Prerequisites

```bash
# Verify Android build tools are installed
cargo mobench check --target android

# Verify iOS build tools are installed
cargo mobench check --target ios

# Validate CI prerequisites + config in one shot
cargo mobench doctor --target both --config bench-config.toml --device-matrix device-matrix.yaml

# Output as JSON for CI parsing
cargo mobench check --target android --format json
```

The `check` command validates:
- Rust toolchain and cargo
- Android: NDK, cargo-ndk, Rust targets, JDK
- iOS: Xcode, xcodegen, Rust targets

### Validate Devices

Before scheduling runs, validate device specs:

```bash
# Validate specific device specs
cargo mobench devices --validate "Google Pixel 7-13.0" "iPhone 14-16"

# List available devices
cargo mobench devices --platform android
cargo mobench devices --platform ios

# Output as JSON
cargo mobench devices --platform android --json
```

Invalid device specs return helpful suggestions:
```
Invalid devices (1):
  [ERROR] Google Pixle 7-13.0: Device not found
          Suggestions:
            - Google Pixel 7-13.0
            - Google Pixel 7 Pro-13.0
```

### Verify Benchmark Setup

```bash
# Verify registry, spec, and artifacts
cargo mobench verify --target android --check-artifacts

# Include smoke test
cargo mobench verify --target android --smoke-test --function my_benchmark
```

## Quick Example

```rust
use mobench::browserstack::{BrowserStackClient, BrowserStackAuth};

// 1. Create client
let client = BrowserStackClient::new(
    BrowserStackAuth {
        username: env::var("BROWSERSTACK_USERNAME")?,
        access_key: env::var("BROWSERSTACK_ACCESS_KEY")?,
    },
    Some("my-project".to_string()),
)?;

// 2. Upload artifacts
let app_upload = client.upload_espresso_app(Path::new("app.apk"))?;
let test_upload = client.upload_espresso_test_suite(Path::new("test.apk"))?;

// 3. Schedule run
let run = client.schedule_espresso_run(
    &["Google Pixel 7-13.0"],
    &app_upload.app_url,
    &test_upload.test_suite_url,
)?;

println!("Build ID: {}", run.build_id);
println!("Dashboard: https://app-automate.browserstack.com/dashboard/v2/builds/{}", run.build_id);

// 4. Wait for completion and fetch results
let (results, _performance) = client.wait_and_fetch_all_results(&run.build_id, "espresso", Some(600))?;

// 5. Process results
for (device, bench_results) in results {
    println!("Device: {}", device);
    for result in bench_results {
        println!("  Result: {}", serde_json::to_string_pretty(&result)?);
    }
}
```

## CLI Integration

### Using `mobench run` with Result Fetching

```bash
# Run and fetch results (use --release for smaller APK, faster uploads)
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 30 \
  --warmup 5 \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --fetch-timeout-secs 600 \
  --ci \
  --output target/mobench/results.json
```

**Note**: Always use the `--release` flag for BrowserStack runs. Debug builds are significantly larger (~544MB vs ~133MB for release) and may cause upload timeouts.

## API Methods

### 1. Poll for Build Completion

```rust
pub fn poll_build_completion(
    &self,
    build_id: &str,
    platform: &str,        // "espresso" or "xcuitest"
    timeout_secs: u64,     // Max wait time
    poll_interval_secs: u64, // Check interval
) -> Result<BuildStatus>
```

**Example:**
```rust
let status = client.poll_build_completion(
    "88f8c5a3134562b8a92004582b757468ee10d08c",
    "espresso",
    600,  // 10 minute timeout
    10,   // Check every 10 seconds
)?;

println!("Build status: {}", status.status);
println!("Duration: {:?}s", status.duration);
```

### 2. Get Build Status (Single Check)

```rust
// For Espresso
let status = client.get_espresso_build_status(build_id)?;

// For XCUITest
let status = client.get_xcuitest_build_status(build_id)?;
```

**Build Status Values:**
- `"running"` - Tests are executing
- `"done"` / `"passed"` / `"completed"` - Tests completed successfully
- `"failed"` - Tests failed
- `"error"` - Build error occurred
- `"timeout"` - Exceeded time limit

### 3. Fetch Device Logs

```rust
let logs = client.get_device_logs(build_id, session_id, "espresso")?;
println!("Device logs:\n{}", logs);
```

### 4. Extract Benchmark Results

```rust
let logs = client.get_device_logs(build_id, session_id, "espresso")?;
let results = client.extract_benchmark_results(&logs)?;

for result in results {
    if let Some(function) = result.get("function") {
        println!("Function: {}", function);
    }
    if let Some(samples) = result.get("samples").and_then(|s| s.as_array()) {
        println!("Samples: {} measurements", samples.len());
    }
}
```

### 5. Complete Workflow (Convenience Method)

```rust
use std::collections::HashMap;

let (results, _performance) = client.wait_and_fetch_all_results(
    build_id,
    "espresso",
    Some(600), // 10 minute timeout
)?;

// Results is a map: device name -> benchmark results
for (device, bench_results) in results {
    println!("\nDevice: {}", device);
    for result in bench_results {
        // Parse benchmark data
        if let Some(mean) = result.get("mean_ns") {
            println!("  Mean: {} ns", mean);
        }
    }
}
```

## GitHub Actions Example

```yaml
name: Mobile Benchmarks

on:
  push:
    branches: [main]
  pull_request:

jobs:
  benchmark-android:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install mobench
        run: cargo install mobench

      - name: Check prerequisites
        run: cargo mobench check --target android

      - name: Validate devices
        env:
          BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
          BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
        run: cargo mobench devices --validate "Google Pixel 7-13.0"

      - name: Run benchmarks on BrowserStack
        env:
          BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
          BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
        run: |
          # Build and run (fetches results)
          # Use --release to reduce APK size and prevent upload timeouts
          cargo mobench run \
            --target android \
            --function my_crate::my_benchmark \
            --iterations 30 \
            --warmup 5 \
            --devices "Google Pixel 7-13.0" \
            --release \
            --fetch \
            --fetch-timeout-secs 600 \
            --output results.json

      - name: Display summary
        run: cargo mobench summary results.json

      - name: Extract metrics
        run: |
          # JSON format for programmatic access
          cargo mobench summary results.json --format json > metrics.json
          cat metrics.json | jq '.[0].mean_ns'

      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: |
            results.json
            metrics.json
```

## Advanced: Custom Result Processing

```rust
use mobench::browserstack::BrowserStackClient;

fn process_benchmark_results(
    client: &BrowserStackClient,
    build_id: &str,
    platform: &str,
) -> Result<()> {
    // Wait for completion
    let status = client.poll_build_completion(build_id, platform, 600, 10)?;

    // Process each device
    for device in &status.devices {
        println!("Processing device: {}", device.device);

        // Fetch logs
        let logs = client.get_device_logs(build_id, &device.session_id, platform)?;

        // Extract results
        let results = client.extract_benchmark_results(&logs)?;

        // Custom analysis
        for result in results {
            if let Some(samples) = result.get("samples").and_then(|s| s.as_array()) {
                let durations: Vec<f64> = samples
                    .iter()
                    .filter_map(|s| s.get("duration_ns")?.as_f64())
                    .collect();

                let mean = durations.iter().sum::<f64>() / durations.len() as f64;
                let min = durations.iter().fold(f64::INFINITY, |a, &b| a.min(b));
                let max = durations.iter().fold(f64::NEG_INFINITY, |a, &b| a.max(b));

                println!("  Mean: {:.2} ns", mean);
                println!("  Min:  {:.2} ns", min);
                println!("  Max:  {:.2} ns", max);
            }
        }
    }

    Ok(())
}
```

## Timeout Recommendations

- **Default**: 600 seconds (10 minutes)
- **Quick tests**: 300 seconds (5 minutes)
- **Extensive benchmarks**: 1200-1800 seconds (20-30 minutes)

## Error Handling

```rust
match client.wait_and_fetch_all_results(build_id, "espresso", Some(600)) {
    Ok((results, _performance)) => {
        println!("Successfully fetched results from {} devices", results.len());
    }
    Err(e) if e.to_string().contains("Timeout") => {
        eprintln!("Build timed out - may still be running");
        eprintln!("Check dashboard: https://app-automate.browserstack.com/dashboard/v2/builds/{}", build_id);
    }
    Err(e) if e.to_string().contains("failed") => {
        eprintln!("Build failed: {}", e);
    }
    Err(e) => {
        eprintln!("Error fetching results: {}", e);
    }
}
```

## Troubleshooting

### BrowserStack credentials not configured

**Error**:
```
BrowserStack credentials not configured.

Set credentials using one of these methods:

  1. Environment variables:
     export BROWSERSTACK_USERNAME=your_username
     export BROWSERSTACK_ACCESS_KEY=your_access_key

  2. Config file (bench-config.toml):
     [browserstack]
     app_automate_username = "your_username"
     app_automate_access_key = "your_access_key"

  3. .env.local file in project root:
     BROWSERSTACK_USERNAME=your_username
     BROWSERSTACK_ACCESS_KEY=your_access_key

Get credentials: https://app-automate.browserstack.com/
(Navigate to Settings -> Access Key)
```

**Solution**: Set credentials using any of the three methods shown.

### Device spec validation failed

**Error**:
```
Invalid devices (1):
  [ERROR] Google Pixle 7-13.0: Device not found
          Suggestions:
            - Google Pixel 7-13.0
            - Google Pixel 7 Pro-13.0
```

**Solution**: Use the suggested device name or run `cargo mobench devices` to see all available devices.

### No benchmark results found

**Cause**: The benchmark app didn't log results, or logs are in unexpected format.

**Solution**:
1. Check device logs manually in BrowserStack dashboard
2. Verify your app logs benchmark results as JSON to stdout/logcat
3. Use `client.get_device_logs()` to inspect raw logs
4. Run `cargo mobench verify --smoke-test` to test locally first

### Build stuck in "running" state

**Cause**: App crashed, tests hung, or device disconnected.

**Solution**:
1. Check the BrowserStack dashboard for device screenshots/video
2. Increase timeout if benchmarks legitimately take longer
3. Add health checks to your benchmark code
4. Use `--progress` flag to see detailed progress during runs

### Rate limiting

**Cause**: Too many API requests.

**Solution**:
1. Increase poll interval: `poll_build_completion(id, platform, 600, 30)` (30s interval)
2. Use BrowserStack's webhook notifications instead of polling
3. Check your BrowserStack plan limits

### Prerequisites missing

**Error**: Build fails with missing tools.

**Solution**: Run `cargo mobench check --target <android|ios>` to identify missing prerequisites and get fix suggestions.

## New CLI Commands

### `cargo mobench check`

Validate prerequisites before building:

```bash
cargo mobench check --target android [--format text|json]
```

### `cargo mobench devices`

List and validate BrowserStack devices:

```bash
# List all devices
cargo mobench devices

# List by platform
cargo mobench devices --platform android

# Validate specific specs
cargo mobench devices --validate "Google Pixel 7-13.0" "iPhone 14-16"

# JSON output
cargo mobench devices --json
```

### `cargo mobench verify`

Validate benchmark setup:

```bash
cargo mobench verify \
  --target android \
  --check-artifacts \
  --smoke-test \
  --function my_benchmark
```

### `cargo mobench summary`

Display statistics from results:

```bash
cargo mobench summary results.json [--format text|json|csv]
```

## Next Steps

- See `BROWSERSTACK_METRICS.md` for metrics and performance documentation
- See `FETCH_RESULTS_GUIDE.md` for detailed fetch and summary workflows
- Check `crates/mobench/src/browserstack.rs` for full API documentation
- Run `cargo doc --open -p mobench` for detailed API docs
