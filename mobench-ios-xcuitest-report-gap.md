# mobench iOS XCUITest Benchmark Report Gap

**Date:** 2026-01-20
**Priority:** P1 - Feature gap preventing iOS benchmark data collection
**Affects:** `mobench fetch` for iOS builds

---

## Summary

The iOS XCUITest template does not capture benchmark results like the Android Espresso test does. After running benchmarks on BrowserStack, Android returns a complete `bench-report.json` with timing data, while iOS returns nothing.

---

## Current Behavior

### Android (Working) ✅

The Android Espresso test:
1. Launches the app
2. Waits for the benchmark to **complete**
3. Extracts the benchmark JSON from the app
4. Saves `bench-report.json` with full results

**Result from `mobench fetch`:**
```
session-xxx/
├── bench-report.json      ✅ Contains timing data
├── session.json
├── device_log.log
├── instrumentation_log.log
└── video.log
```

**bench-report.json contents:**
```json
{
  "spec": {
    "name": "bench_mobile::bench_query_proof_generation",
    "iterations": 20,
    "warmup": 3
  },
  "samples_ns": [395761841, 404437540, ...],
  "stats": {
    "avg_ns": 415465220.25,
    "min_ns": 389723715,
    "max_ns": 459591756
  },
  "resources": {
    "elapsed_cpu_ms": 52129,
    "java_heap_kb": 2770,
    "total_pss_kb": 71489
  }
}
```

### iOS (Not Working) ❌

The iOS XCUITest only checks that the UI exists:

**Current template** (`BenchRunnerUITests.swift`):
```swift
final class BenchRunnerUITests: XCTestCase {
    func testLaunchShowsBenchmarkReport() {
        let app = XCUIApplication()
        app.launch()

        let report = app.staticTexts["benchmarkReport"]
        let exists = report.waitForExistence(timeout: 30.0)
        XCTAssertTrue(exists, "Benchmark report text should appear after launch")
    }
}
```

**Problems:**
1. Only waits for element to **exist** (2 seconds), not for benchmark to **complete** (could be 30+ seconds)
2. Does not extract the benchmark report text
3. Does not save any JSON output

**Result from `mobench fetch`:**
```
session-xxx/
├── session.json
├── device_log.log          (empty)
├── instrumentation_log.log
└── video.log
                            ❌ NO bench-report.json
```

---

## Evidence from BrowserStack Logs

**iOS instrumentation log:**
```
t =     1.19s Waiting 30.0s for "benchmarkReport" StaticText to exist
t =     2.20s     Checking existence of `"benchmarkReport" StaticText`
t =     2.23s Tear Down
Test Case passed (2.429 seconds).
```

The test found the element in 2.2 seconds and exited immediately. The actual benchmark (which takes 400ms × 20 iterations = 8+ seconds minimum) never ran or was ignored.

**Android instrumentation log (for comparison):**
- Session duration: **50 seconds**
- Captured 20 benchmark samples
- Full resource metrics

---

## Required Changes

### 1. Update iOS XCUITest Template

The test needs to:

```swift
final class BenchRunnerUITests: XCTestCase {
    func testLaunchAndCaptureBenchmarkReport() {
        let app = XCUIApplication()
        app.launch()

        // 1. Wait for benchmark to COMPLETE (not just start)
        //    Look for a "completed" indicator or specific text pattern
        let completedIndicator = app.staticTexts["benchmarkCompleted"]
        let completed = completedIndicator.waitForExistence(timeout: 300.0)  // 5 min timeout
        XCTAssertTrue(completed, "Benchmark should complete")

        // 2. Extract the benchmark report JSON from the UI
        let reportText = app.staticTexts["benchmarkReportJSON"]
        XCTAssertTrue(reportText.exists, "Benchmark report JSON should exist")

        // 3. Log it in a format that mobench fetch can parse
        let jsonString = reportText.label
        print("BENCH_REPORT_JSON_START")
        print(jsonString)
        print("BENCH_REPORT_JSON_END")

        // Or write to a known file location that BrowserStack can retrieve
    }
}
```

### 2. Update iOS App to Expose Report Data

The `BenchRunner` iOS app needs to:
1. Show a "completed" indicator when benchmark finishes
2. Expose the full JSON report in an accessible UI element (or via `XCUIApplication.launchArguments` output)

### 3. Update `mobench fetch` for iOS

Parse the instrumentation log or device log for the benchmark JSON:
```rust
// In fetch command for iOS
fn extract_ios_bench_report(instrumentation_log: &str) -> Option<BenchReport> {
    // Look for BENCH_REPORT_JSON_START ... BENCH_REPORT_JSON_END
    // Parse and save as bench-report.json
}
```

---

## Comparison: Android vs iOS Test Flow

| Step | Android (Espresso) | iOS (XCUITest) Current | iOS (XCUITest) Required |
|------|-------------------|------------------------|------------------------|
| Launch app | ✅ | ✅ | ✅ |
| Wait for benchmark complete | ✅ Waits for completion | ❌ Only checks existence | ✅ Need to wait |
| Extract JSON report | ✅ Via Espresso | ❌ Not implemented | ✅ Via XCUITest |
| Save bench-report.json | ✅ Automatic | ❌ Missing | ✅ Need to implement |
| Duration | 50 seconds | 2 seconds | Should be ~50 seconds |

---

## Files to Modify

1. **`crates/mobench-sdk/templates/ios/BenchRunnerUITests/BenchRunnerUITests.swift`**
   - Rewrite test to wait for completion and extract report

2. **`crates/mobench-sdk/templates/ios/BenchRunner/ContentView.swift`** (or equivalent)
   - Add accessibility identifiers for completed state and JSON report

3. **`crates/mobench/src/fetch.rs`** (or equivalent)
   - Add iOS-specific bench report extraction from logs

---

## Test Plan

After fix:
```bash
# Build and run iOS benchmark
mobench build --target ios --release
mobench package-ipa
mobench package-xcuitest
mobench run --target ios --devices "iPhone 15-17" --function bench_mobile::bench_query_proof_generation

# Fetch should now include bench-report.json
mobench fetch --target ios --build-id <id> --wait

# Verify
ls target/browserstack/<build-id>/session-*/bench-report.json
cat target/browserstack/<build-id>/session-*/bench-report.json
# Should contain: samples_ns, stats.avg_ns, stats.min_ns, stats.max_ns, etc.
```

---

## Acceptance Criteria

- [ ] iOS XCUITest waits for benchmark to complete (not just UI element existence)
- [ ] iOS XCUITest extracts full benchmark JSON
- [ ] `mobench fetch --target ios` produces `bench-report.json` with same structure as Android
- [ ] Benchmark timing data (samples_ns, stats) is captured
- [ ] Resource usage data is captured (if available on iOS)
