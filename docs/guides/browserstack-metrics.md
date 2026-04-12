# BrowserStack Device Metrics

This document describes what device metrics BrowserStack provides and what we currently capture.

This guide is about BrowserStack benchmark metrics, not native profiling. In
the current release, BrowserStack remains a benchmark execution target for
timing plus resource metrics. Native flamegraph/native stack capture is still
local-only.

## Current Implementation

### ✅ What We Capture Now

**Build-level:**
- Build ID
- Build status (running, done, passed, completed, failed, error, timeout)
- Build duration (total time in seconds)

**Session-level:**
- Device name (e.g., "Google Pixel 7-13.0")
- Session ID (for artifact URLs)
- Session status (passed, failed, error)
- Device logs (text, contains logcat/system logs)

**Artifacts Downloaded:**
We recursively download ALL URLs from session JSON, which typically includes:
- `device_logs` - Device logcat/console output
- `video_url` - Screen recording of test execution
- `appium_logs_url` - Appium automation logs
- `instrumentation_logs` - Espresso/XCUITest instrumentation logs
- `network_logs` - Network traffic logs (HAR format)
- `screenshots` - Screenshots at various test points

**Benchmark Data:**
- Extracted from device logs (JSON output from your app)
- Timing samples (duration_ns for each iteration)
- Statistical metrics (mean, median, min, max, stddev)

**Performance Metrics:**
- Extracted from device logs (JSON output with `"type": "performance"` or `memory`/`cpu` fields)
- Enriched from BrowserStack App Profiling v2 when that API returns additional session data
- Memory usage (used_mb, max_mb, available_mb, total_mb)
  - Aggregate statistics: peak, average, min
- CPU usage (usage_percent)
  - Aggregate statistics: peak, average, min
- Normalized into run summaries and CI summaries when using `--fetch`
- Surfaced as summary resource fields such as `cpu_total_ms` and `peak_memory_kb`

**Benchmark-scoped resource metrics (current default):**
- Each measured iteration can emit `cpu_time_ms` and `peak_memory_kb`
- `mobench` derives `cpu_median_ms` from measured iterations only
- `summary.md` renders the default `CPU` column from `cpu_median_ms` in milliseconds below one second and total seconds otherwise
- `results.csv` includes `cpu_total_ms`, `cpu_median_ms`, and `peak_memory_kb`
- BrowserStack aggregate memory is only used as a fallback when benchmark-scoped peak memory is absent

### ⚠️ What We do not currently capture

Based on [BrowserStack App Automate API documentation](https://www.browserstack.com/docs/app-automate/api-reference):

**Session Details (available but not parsed):**
- `duration` - Individual session duration (vs. total build duration)
- `start_time` / `end_time` - Precise timestamps
- `app_details` - App name, version, custom_id
- `reason` - Failure reason if test failed
- `build_tag` - Custom build tags

**Performance metrics:**

BrowserStack does not expose CPU/memory/battery metrics in the normal session
JSON payloads. mobench therefore combines two sources when you use `--fetch`:

1. **Collect metrics in your app** using Android/iOS APIs:
   - Android: `ActivityManager.MemoryInfo`, `Debug.MemoryInfo`
   - iOS: `task_info`, `mach_task_basic_info`

2. **Log to device logs** in JSON format (see example below)

3. **mobench automatically extracts** them alongside benchmark results and merges in BrowserStack App Profiling v2 data when available

## BrowserStack limitations

According to their documentation, BrowserStack App Automate **does not** provide:

- ❌ Built-in memory profiling
- ❌ Built-in CPU profiling
- ❌ Built-in battery/power profiling
- ❌ Built-in frame rate/rendering metrics

These metrics must either be collected by **your application code** and logged,
or enriched from BrowserStack App Profiling v2 when that endpoint has data for a
session. Neither path provides native stacks or flamegraphs.

## How to Add Performance Metrics

### Step 1: Collect Metrics in Your App

**Android Example:**
```kotlin
fun getMemoryUsage(): MemoryMetrics {
    val runtime = Runtime.getRuntime()
    val activityManager = getSystemService(Context.ACTIVITY_SERVICE) as ActivityManager
    val memInfo = ActivityManager.MemoryInfo()
    activityManager.getMemoryInfo(memInfo)

    return MemoryMetrics(
        usedMemoryMB = (runtime.totalMemory() - runtime.freeMemory()) / 1_048_576,
        maxMemoryMB = runtime.maxMemory() / 1_048_576,
        availableMemoryMB = memInfo.availMem / 1_048_576,
        totalMemoryMB = memInfo.totalMem / 1_048_576
    )
}

// In your benchmark code:
val beforeMemory = getMemoryUsage()
runBenchmark(spec)
val afterMemory = getMemoryUsage()

// Log as JSON
println("""
    {"type":"performance","timestamp":${System.currentTimeMillis()},"memory":{"before":${beforeMemory},"after":${afterMemory}}}
""".trimIndent())
```

**iOS Example:**
```swift
func getMemoryUsage() -> MemoryMetrics {
    var info = mach_task_basic_info()
    var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size)/4
    let kerr: kern_return_t = withUnsafeMutablePointer(to: &info) {
        $0.withMemoryRebound(to: integer_t.self, capacity: 1) {
            task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
        }
    }

    return MemoryMetrics(
        residentSizeMB: Double(info.resident_size) / 1_048_576,
        virtualSizeMB: Double(info.virtual_size) / 1_048_576
    )
}
```

### Step 2: Log Metrics in Structured Format

Ensure your app logs performance metrics as JSON to stdout/logcat:

```json
{
  "type": "performance_snapshot",
  "timestamp_ms": 1705238400000,
  "memory": {
    "used_mb": 128.5,
    "max_mb": 512.0,
    "available_mb": 383.5
  },
  "cpu": {
    "usage_percent": 45.2
  }
}
```

### Step 3: Extract Metrics with mobench

mobench automatically extracts both benchmark results and performance metrics:

```rust
// Extracts benchmark results
let results = client.extract_benchmark_results(&logs)?;

// Extracts performance metrics
let performance = client.extract_performance_metrics(&logs)?;

// Or get both at once
let (bench_results, perf_metrics) = client.wait_and_fetch_all_results(
    build_id,
    platform,
    Some(timeout_secs),
)?;
```

## Implementation Details

### API Methods

```rust
impl BrowserStackClient {
    /// Extract performance metrics from device logs
    pub fn extract_performance_metrics(&self, logs: &str) -> Result<PerformanceMetrics>;

    /// Wait for build completion and fetch both benchmark and performance results
    pub fn wait_and_fetch_all_results(
        &self,
        build_id: &str,
        platform: &str,
        timeout_secs: Option<u64>,
    ) -> Result<(
        HashMap<String, Vec<Value>>,        // benchmark_results
        HashMap<String, PerformanceMetrics>, // performance_metrics
    )>;
}

pub struct PerformanceMetrics {
    pub sample_count: usize,
    pub memory: Option<AggregateMemoryMetrics>,
    pub cpu: Option<AggregateCpuMetrics>,
    pub snapshots: Vec<PerformanceSnapshot>,
}

pub struct AggregateMemoryMetrics {
    pub peak_mb: f64,
    pub average_mb: f64,
    pub min_mb: f64,
}

pub struct AggregateCpuMetrics {
    pub peak_percent: f64,
    pub average_percent: f64,
    pub min_percent: f64,
}
```

### RunSummary Output Format

```json
{
  "spec": {...},
  "benchmark_results": {
    "Google Pixel 7-13.0": [...]
  },
  "performance_metrics": {
    "Google Pixel 7-13.0": {
      "sample_count": 30,
      "memory": {
        "peak_mb": 145.2,
        "average_mb": 128.7,
        "min_mb": 115.3
      },
      "cpu": {
        "peak_percent": 78.5,
        "average_percent": 45.2,
        "min_percent": 12.1
      },
      "snapshots": [
        {
          "timestamp_ms": 1705238400000,
          "memory": {"used_mb": 128.5, "max_mb": 512.0},
          "cpu": {"usage_percent": 45.2}
        }
      ]
    }
  }
}
```

## Third-Party Performance Tools

For more comprehensive profiling, consider:

1. **Android Profiler** (Android Studio)
   - Requires USB connection, not available on BrowserStack

2. **Instruments** (Xcode)
   - Requires Mac + physical device, not available on BrowserStack

3. **Firebase Performance Monitoring**
   - Can work on BrowserStack devices
   - Requires Firebase SDK integration
   - Provides CPU, memory, network metrics

4. **Custom Instrumentation**
   - Most flexible for BrowserStack
   - Log metrics as JSON from your app
   - Extract with mobench CLI

## Recommendations

For CI/benchmarking on BrowserStack:

1. **Implement custom metric collection** in your app for the metrics you care about most
2. **Log metrics as JSON** to stdout/logcat so `mobench` can extract them deterministically
3. **Use `mobench --fetch`** so device logs and App Profiling v2 enrichment both contribute to the final summary
4. **Focus on metrics that matter** for your use case:
   - Memory: Peak usage, allocations during benchmark
   - CPU: Usage spikes during computation
   - Time: Already well-captured by benchmark harness

## Manual Inspection

If you need to inspect raw logs, you can still parse them directly:

```bash
cargo mobench run --fetch --output results.json
grep '"type":"performance"' target/browserstack/*/session-*/device-logs.txt | jq .
```

## See Also

- BrowserStack API Docs: https://www.browserstack.com/docs/app-automate/api-reference
- Android MemoryInfo: https://developer.android.com/reference/android/app/ActivityManager.MemoryInfo
- iOS Memory Profiling: https://developer.apple.com/documentation/foundation/task_management
- [browserstack-ci.md](browserstack-ci.md): benchmark execution and CI usage
- [fetch-results.md](fetch-results.md): fetch/output flow and artifact layout
