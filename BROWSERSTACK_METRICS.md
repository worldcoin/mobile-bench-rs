# BrowserStack Device Metrics

This document describes what device metrics BrowserStack provides and what we currently capture.

## Current Implementation

### ✅ What We Capture Now

**Build-level:**
- Build ID
- Build status (running, done, failed, error, timeout)
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

### ⚠️ What We DON'T Capture (But BrowserStack Provides)

Based on [BrowserStack App Automate API documentation](https://www.browserstack.com/docs/app-automate/api-reference):

**Session Details (available but not parsed):**
- `duration` - Individual session duration (vs. total build duration)
- `start_time` / `end_time` - Precise timestamps
- `app_details` - App name, version, custom_id
- `reason` - Failure reason if test failed
- `build_tag` - Custom build tags

**Performance Metrics (Requires Separate API Calls):**

BrowserStack does NOT provide built-in CPU/Memory/Battery metrics in standard API responses. These would need to be:

1. **Collected by your app** using Android/iOS APIs:
   - Android: `ActivityManager.MemoryInfo`, `Debug.MemoryInfo`
   - iOS: `task_info`, `mach_task_basic_info`

2. **Logged to device logs** in JSON format

3. **Extracted by our tool** alongside benchmark results

## BrowserStack Limitations

According to their documentation, BrowserStack App Automate **does not** provide:

- ❌ Built-in memory profiling
- ❌ Built-in CPU profiling
- ❌ Built-in battery/power profiling
- ❌ Built-in frame rate/rendering metrics

These metrics must be collected by **your application code** and logged.

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

The `extract_benchmark_results()` function can be enhanced to also extract performance metrics:

```rust
// Current: Only extracts benchmark results
let results = client.extract_benchmark_results(&logs)?;

// Future: Also extract performance metrics
let performance = client.extract_performance_metrics(&logs)?;
```

## Proposed Enhancement

Add performance metric extraction to mobench:

### New API Methods

```rust
impl BrowserStackClient {
    /// Extract performance metrics from device logs
    pub fn extract_performance_metrics(&self, logs: &str) -> Result<Vec<PerformanceSnapshot>>;
}

pub struct PerformanceSnapshot {
    pub timestamp_ms: u64,
    pub memory: MemoryMetrics,
    pub cpu: Option<CpuMetrics>,
}

pub struct MemoryMetrics {
    pub used_mb: f64,
    pub max_mb: f64,
    pub available_mb: Option<f64>,
}
```

### Enhanced RunSummary Output

```json
{
  "spec": {...},
  "benchmark_results": {
    "Google Pixel 7-13.0": [...]
  },
  "performance_metrics": {
    "Google Pixel 7-13.0": {
      "memory": {
        "peak_mb": 145.2,
        "average_mb": 128.7,
        "samples": 30
      },
      "cpu": {
        "peak_percent": 78.5,
        "average_percent": 45.2
      }
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

1. **Implement custom metric collection** in your app
2. **Log metrics as JSON** to stdout/logcat
3. **Extend mobench** to extract performance metrics (future enhancement)
4. **Focus on metrics that matter** for your use case:
   - Memory: Peak usage, allocations during benchmark
   - CPU: Usage spikes during computation
   - Time: Already well-captured by benchmark harness

## Current Workaround

Until performance metric extraction is built-in:

1. Log performance metrics from your app as JSON
2. Use `--fetch` to download device logs
3. Manually parse performance data from logs:

```bash
cargo mobench run --fetch --output results.json
grep '"type":"performance"' target/browserstack/*/session-*/device-logs.txt | jq .
```

## See Also

- BrowserStack API Docs: https://www.browserstack.com/docs/app-automate/api-reference
- Android MemoryInfo: https://developer.android.com/reference/android/app/ActivityManager.MemoryInfo
- iOS Memory Profiling: https://developer.apple.com/documentation/foundation/task_management
