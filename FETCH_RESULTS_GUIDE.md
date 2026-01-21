# Fetching BrowserStack Results in CI

This guide shows how to use the `--fetch` flag to wait for BrowserStack tests to complete and retrieve results in CI pipelines.

## Quick Start

The `--fetch` flag makes `cargo mobench run` wait for BrowserStack tests to complete and automatically fetch benchmark results:

```bash
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --iterations 30 \
  --warmup 5 \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --output results.json
```

**Note**: Always use the `--release` flag for BrowserStack runs. Debug builds are significantly larger (~544MB vs ~133MB for release) and may cause upload timeouts.

## How It Works

When `--fetch` is enabled:

1. **Builds and uploads** artifacts to BrowserStack
2. **Schedules** test run on specified devices
3. **Polls** for build completion (checks every 5 seconds)
4. **Fetches** device logs from all sessions
5. **Extracts** benchmark results as JSON
6. **Merges** results into output file

## Output Format

With `--fetch`, the output JSON includes a `benchmark_results` field:

```json
{
  "spec": {
    "target": "android",
    "function": "sample_fns::fibonacci",
    "iterations": 30,
    "warmup": 5,
    "devices": ["Google Pixel 7-13.0"]
  },
  "remote_run": {
    "platform": "android",
    "app_url": "bs://...",
    "build_id": "88f8c5a..."
  },
  "benchmark_results": {
    "Google Pixel 7-13.0": [
      {
        "function": "sample_fns::fibonacci",
        "iterations": 30,
        "warmup": 5,
        "samples": [
          {"duration_ns": 1234000},
          {"duration_ns": 1240000},
          ...
        ],
        "mean_ns": 1237000,
        "median_ns": 1236500,
        "min_ns": 1230000,
        "max_ns": 1245000
      }
    ]
  }
}
```

## Configuration Options

### Timeout

Control how long to wait for build completion (default: 300 seconds / 5 minutes):

```bash
cargo mobench run \
  --target android \
  --function my_func \
  --devices "..." \
  --fetch \
  --fetch-timeout-secs 600  # Wait up to 10 minutes
```

### Poll Interval

Control how often to check build status (default: 5 seconds):

```bash
cargo mobench run \
  --target android \
  --function my_func \
  --devices "..." \
  --fetch \
  --fetch-poll-interval-secs 30  # Check every 30 seconds
```

### Output Directory

Detailed artifacts (logs, screenshots, videos) are saved separately:

```bash
cargo mobench run \
  --target android \
  --function my_func \
  --devices "..." \
  --fetch \
  --fetch-output-dir target/browserstack  # Default location
```

Directory structure:
```
target/browserstack/
└── {build_id}/
    ├── build.json              # Build metadata
    ├── sessions.json           # Session list
    └── session-{id}/
        ├── session.json        # Session details
        ├── bench-report.json   # Extracted benchmark data
        ├── device-logs.txt     # Raw device logs
        └── *.mp4, *.png        # Videos and screenshots
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
    timeout-minutes: 45
    steps:
      - uses: actions/checkout@v4

      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Setup Android NDK
        uses: android-actions/setup-android@v3
        with:
          packages: ndk;26.1.10909125

      - name: Install mobench
        run: cargo install mobench

      - name: Run benchmarks on BrowserStack
        env:
          BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
          BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
          ANDROID_NDK_HOME: /usr/local/lib/android/sdk/ndk/26.1.10909125
        run: |
          # Use --release to reduce APK size and prevent upload timeouts
          cargo mobench run \
            --target android \
            --function my_crate::my_benchmark \
            --iterations 30 \
            --warmup 5 \
            --devices "Google Pixel 7-13.0" \
            --release \
            --fetch \
            --fetch-timeout-secs 900 \
            --output results.json

      - name: Extract metrics
        run: |
          echo "## Benchmark Results" >> $GITHUB_STEP_SUMMARY
          jq -r '.benchmark_results | to_entries[] | "### \(.key)\n- Mean: \(.value[0].mean_ns)ns\n- Min: \(.value[0].min_ns)ns\n- Max: \(.value[0].max_ns)ns"' results.json >> $GITHUB_STEP_SUMMARY

      - name: Upload results
        uses: actions/upload-artifact@v4
        with:
          name: benchmark-results
          path: |
            results.json
            target/browserstack/
```

## Error Handling

### Build Timeout

If the build exceeds the timeout, you'll see:

```
Warning: Failed to fetch benchmark results: Timeout waiting for build 88f8c5a... to complete (waited 600 seconds)
Build may still be accessible at: https://app-automate.browserstack.com/dashboard/v2/builds/88f8c5a...
```

The command will still succeed and write partial results. You can manually check the dashboard or use the separate `fetch` command:

```bash
cargo mobench fetch \
  --target android \
  --build-id 88f8c5a3134562b8a92004582b757468ee10d08c \
  --output-dir target/browserstack
```

### Build Failed

If the test fails on BrowserStack:

```
Warning: Failed to fetch benchmark results: Build 88f8c5a... failed with status: failed
```

Check the dashboard for error details. Common causes:
- App crashed during startup
- Test timed out
- Device disconnected

### No Results Found

If logs don't contain benchmark JSON:

```
Warning: Failed to fetch benchmark results: No benchmark results found in device logs
```

This means:
- Your app didn't log benchmark results
- Logs are in unexpected format
- Tests didn't actually run

Verify your app logs JSON to stdout/logcat in the correct format.

## Best Practices

1. **Always use --fetch in CI** for automated pipelines
2. **Always use --release for BrowserStack** to reduce artifact sizes (~544MB debug vs ~133MB release) and prevent upload timeouts
3. **Set reasonable timeouts** based on your benchmark duration
4. **Check exit codes** - command succeeds even if fetch warns
5. **Archive results** as CI artifacts for historical tracking
6. **Use GitHub Actions summaries** to display results inline

## Comparison with Manual Workflow

### Without --fetch
```bash
# 1. Run and schedule
cargo mobench run --target android --function my_func --devices "..."

# 2. Wait manually
# (check dashboard periodically)

# 3. Fetch later
cargo mobench fetch --target android --build-id <id>

# 4. Parse logs manually
cat target/browserstack/.../device-logs.txt | grep '{"function"'
```

### With --fetch
```bash
# One command does everything (use --release for BrowserStack)
cargo mobench run \
  --target android \
  --function my_func \
  --devices "..." \
  --release \
  --fetch \
  --output results.json

# Results already in results.json!
```

## See Also

- `BROWSERSTACK_CI_INTEGRATION.md` - Programmatic API for custom workflows
- `BROWSERSTACK_METRICS.md` - Metrics and performance documentation
- `cargo mobench run --help` - Full CLI options
