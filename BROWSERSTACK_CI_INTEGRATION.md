# BrowserStack CI Integration Guide

This guide shows how to run benchmarks on BrowserStack and fetch results within CI pipelines.

## Overview

The BrowserStack client now supports:
1. **Scheduling runs** - Upload artifacts and start tests
2. **Polling for completion** - Wait for tests to finish (with timeout)
3. **Fetching results** - Download device logs and extract benchmark data

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
# Run and fetch results
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 30 \
  --warmup 5 \
  --devices "Google Pixel 7-13.0" \
  --fetch \
  --fetch-timeout-secs 600 \
  --output results.json
```

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

      - name: Run benchmarks on BrowserStack
        env:
          BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
          BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
        run: |
          # Build and run (fetches results)
          cargo mobench run \
            --target android \
            --function my_crate::my_benchmark \
            --iterations 30 \
            --warmup 5 \
            --devices "Google Pixel 7-13.0" \
            --fetch \
            --fetch-timeout-secs 600 \
            --output results.json

          # Extract metrics for comparison
          cat results.json | jq '.devices[0].samples | map(.duration_ns) | add / length'

      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: results.json
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

### No benchmark results found

**Cause**: The benchmark app didn't log results, or logs are in unexpected format.

**Solution**:
1. Check device logs manually in BrowserStack dashboard
2. Verify your app logs benchmark results as JSON to stdout/logcat
3. Use `client.get_device_logs()` to inspect raw logs

### Build stuck in "running" state

**Cause**: App crashed, tests hung, or device disconnected.

**Solution**:
1. Check the BrowserStack dashboard for device screenshots/video
2. Increase timeout if benchmarks legitimately take longer
3. Add health checks to your benchmark code

### Rate limiting

**Cause**: Too many API requests.

**Solution**:
1. Increase poll interval: `poll_build_completion(id, platform, 600, 30)` (30s interval)
2. Use BrowserStack's webhook notifications instead of polling
3. Check your BrowserStack plan limits

## Next Steps

- See `BROWSERSTACK_METRICS.md` for metrics and performance documentation
- Check `crates/mobench/src/browserstack.rs` for full API documentation
- Run `cargo doc --open -p mobench` for detailed API docs
