# Native iOS Resource Usage Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add native iOS CPU time and peak memory to benchmark JSON so the existing `mobench` CI summary/report path renders `CPU total (ms)` and `Peak memory` for iOS.

**Architecture:** Collect native process metrics inside the iOS BenchRunner app before and after `runBenchmark(spec:)`, emit them in the existing `resources` JSON, and extend the Rust parser to consume direct iOS `peak_memory_kb`. This keeps the current BrowserStack-run benchmark workflow and summary rendering path unchanged.

**Tech Stack:** Swift, Darwin/Mach task APIs, Rust, serde_json, GitHub Actions

---

### Task 1: Add Rust coverage for direct iOS resource fields

**Files:**
- Modify: `crates/mobench/src/lib.rs`

**Step 1: Write the failing tests**

Add tests that cover:

```rust
#[test]
fn test_extract_resource_usage_prefers_direct_peak_memory_field() {
    let entry = json!({
        "resources": {
            "elapsed_cpu_ms": 482,
            "peak_memory_kb": 249416
        }
    });

    let usage = extract_benchmark_resource_usage(&entry, None).unwrap();
    assert_eq!(usage.cpu_total_ms, Some(482));
    assert_eq!(usage.peak_memory_kb, Some(249416));
}
```

and one summary-building test that uses an iOS-style `resources` payload with direct `peak_memory_kb`.

**Step 2: Run tests to verify they fail**

Run: `cargo test -p mobench test_extract_resource_usage_prefers_direct_peak_memory_field --lib`

Expected: FAIL because direct `peak_memory_kb` is not read yet.

**Step 3: Write the minimal Rust implementation**

Update `extract_benchmark_resource_usage` to read:

```rust
let direct_peak_memory_kb = resources
    .and_then(|res| res.get("peak_memory_kb"))
    .and_then(json_value_to_u64);
```

and prefer that value before BrowserStack/per-platform fallbacks.

**Step 4: Run tests to verify they pass**

Run:
- `cargo test -p mobench test_extract_resource_usage_prefers_direct_peak_memory_field --lib`
- `cargo test -p mobench build_summary_preserves_resource_usage_from_benchmark_results --lib`

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/mobench/src/lib.rs
git commit -m "test: cover direct ios resource fields"
```

### Task 2: Emit native resource metrics from the checked-in iOS BenchRunner

**Files:**
- Modify: `ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift`

**Step 1: Add a resource snapshot helper**

Introduce a helper struct/function that reads:

```swift
struct ProcessResourceSnapshot {
    let cpuTotalMs: UInt64
    let peakMemoryKb: UInt64
}
```

using Darwin/Mach task APIs for cumulative CPU time and high-water resident memory.

**Step 2: Capture start/end snapshots around the benchmark**

Change the benchmark run path from:

```swift
let report = try runBenchmark(spec: spec)
let jsonReport = generateJSONReport(report)
```

to:

```swift
let before = readProcessResourceSnapshot()
let report = try runBenchmark(spec: spec)
let after = readProcessResourceSnapshot()
let jsonReport = generateJSONReport(report, before: before, after: after)
```

**Step 3: Emit the resource fields**

Extend the JSON `resources` object to include:

```swift
"elapsed_cpu_ms": max(after.cpuTotalMs - before.cpuTotalMs, 0),
"peak_memory_kb": after.peakMemoryKb
```

alongside the existing `platform` and `timestamp_ms`.

**Step 4: Verify the file still builds cleanly through the generated project path**

Run a project-appropriate validation command after the Rust side is ready. Prefer a repo-native command that exercises the iOS build path.

**Step 5: Commit**

```bash
git add ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift
git commit -m "feat: emit native ios benchmark resource metrics"
```

### Task 3: Sync the generator template with the checked-in iOS app

**Files:**
- Modify: `crates/mobench-sdk/templates/ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift.template`

**Step 1: Mirror the checked-in BenchRunner changes**

Copy the same resource snapshot helper, benchmark wrapper changes, and JSON emission logic into the template file.

**Step 2: Compare the checked-in file and template**

Run a diff-oriented command to verify they stay logically aligned in the resource section.

**Step 3: Commit**

```bash
git add crates/mobench-sdk/templates/ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift.template
git commit -m "feat: sync ios bench template resource metrics"
```

### Task 4: Add summary/render regression coverage

**Files:**
- Modify: `crates/mobench/src/lib.rs`

**Step 1: Add end-to-end summary tests for iOS-style payloads**

Cover a benchmark result entry shaped like:

```rust
json!({
    "function": "sample_fns::fibonacci",
    "mean_ns": 100_000_000.0,
    "median_ns": 100_000_000.0,
    "min_ns": 95_000_000.0,
    "max_ns": 120_000_000.0,
    "resources": {
        "elapsed_cpu_ms": 482,
        "peak_memory_kb": 249416
    }
})
```

and assert the resulting summary contains both fields.

**Step 2: Run focused tests**

Run:
- `cargo test -p mobench build_summary_preserves_resource_usage_from_benchmark_results --lib`
- `cargo test -p mobench render_markdown_summary_includes_resource_usage_columns_when_present --lib`
- `cargo test -p mobench resource_usage_tests --lib`

Expected: PASS.

**Step 3: Commit**

```bash
git add crates/mobench/src/lib.rs
git commit -m "test: cover ios native resource usage in summaries"
```

### Task 5: End-to-end verification and CI validation

**Files:**
- Modify if needed: `crates/mobench/README.md`

**Step 1: Run the focused local verification set**

Run:
- `cargo test -p mobench resource_usage_tests --lib`
- `cargo test -p mobench build_summary_preserves_resource_usage_from_benchmark_results --lib`
- `cargo test -p mobench render_markdown_summary_includes_resource_usage_columns_when_present --lib`

**Step 2: If macOS tooling is available, run an iOS-native smoke check**

Prefer a repo-native command that exercises the iOS local path or the example benchmark path.

**Step 3: Push the branch and dispatch the benchmark workflow**

Use the same example workflow path that already proved the Android-side summary rendering change.

**Step 4: Confirm CI artifacts/tables**

Verify iOS `summary.json` now includes `resource_usage.cpu_total_ms` and `resource_usage.peak_memory_kb`, and confirm the rendered markdown table shows those columns.

**Step 5: Commit docs if behavior documentation changed**

```bash
git add crates/mobench/README.md
git commit -m "docs: note ios native resource metrics"
```
