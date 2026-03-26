# ENG-25 Profiling Feature Set Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a new `mobench profile` subsystem that standardizes profiling orchestration and artifacts across platforms, provides native-tool backends for Android and iOS local runs, and reports explicit capability limits for BrowserStack-backed execution.

**Architecture:** Keep the `mobench` CLI as the orchestrator. Introduce a dedicated `profile` Rust module tree for command parsing, artifact contracts, backend dispatch, and summary rendering. Android and iOS backends build native command lines for external tools, write run-scoped `target/mobench/profile/<run-id>/profile.json` manifests plus top-level latest-session convenience files, and preserve raw platform-specific artifacts without changing the existing CI v1 benchmark contract.

**Tech Stack:** Rust (`clap`, `serde`, `serde_json`, `anyhow`, `std::process`, `std::fs`), existing `mobench-sdk` builders, external platform tools (`adb`, `simpleperf`, `xcrun`, `xctrace`), existing summary/report patterns under `crates/mobench/src`.

---

### Task 1: Add Profile CLI Parsing And Contract Types

**Files:**
- Create: `crates/mobench/src/profile.rs`
- Modify: `crates/mobench/src/lib.rs`
- Test: `crates/mobench/src/profile.rs`

**Step 1: Write the failing test**

Add parser-focused tests in `crates/mobench/src/profile.rs` that assert the new
subcommand shape parses cleanly:

```rust
#[test]
fn profile_run_parses_with_android_backend() {
    let cli = Cli::try_parse_from([
        "mobench",
        "profile",
        "run",
        "--target",
        "android",
        "--function",
        "sample_fns::fibonacci",
        "--backend",
        "android-native",
    ])
    .expect("parse profile run");

    assert!(matches!(cli.command, Command::Profile { .. }));
}

#[test]
fn profile_summarize_parses_with_default_profile_path() {
    let cli = Cli::try_parse_from(["mobench", "profile", "summarize"])
        .expect("parse profile summarize");

    assert!(matches!(cli.command, Command::Profile { .. }));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mobench profile_run_parses_with_android_backend -- --exact`

Expected: FAIL because `Command::Profile` and the profile argument types do not
exist yet.

**Step 3: Write minimal implementation**

Create `crates/mobench/src/profile.rs` with the shared enums and manifest types:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum ProfileBackend {
    Auto,
    AndroidNative,
    IosInstruments,
    RustTracing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
pub enum ProfileFormat {
    Native,
    Processed,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileManifest {
    pub run_id: String,
    pub target: String,
    pub function: String,
    pub backend: ProfileBackend,
    pub raw_artifacts: Vec<PathBuf>,
    pub processed_artifacts: Vec<PathBuf>,
    pub warnings: Vec<String>,
}
```

Wire the module into `crates/mobench/src/lib.rs`:

```rust
mod profile;
```

Then add `Command::Profile` plus `ProfileCommand::{Run,Summarize}` with a first
pass argument structure.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mobench profile_run_parses_with_android_backend -- --exact`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/mobench/src/lib.rs crates/mobench/src/profile.rs
git commit -m "feat: add profile command scaffolding"
```

### Task 2: Add Normalized Profile Manifest Rendering

**Files:**
- Modify: `crates/mobench/src/profile.rs`
- Test: `crates/mobench/src/profile.rs`

**Step 1: Write the failing test**

Add a manifest round-trip test and summary-render test:

```rust
#[test]
fn profile_manifest_serializes_partial_failure_state() {
    let manifest = ProfileManifest {
        run_id: "run-123".into(),
        target: "android".into(),
        function: "sample_fns::fibonacci".into(),
        backend: ProfileBackend::AndroidNative,
        raw_artifacts: vec![PathBuf::from("artifacts/raw/sample.perf")],
        processed_artifacts: vec![],
        warnings: vec!["missing symbols".into()],
    };

    let json = serde_json::to_value(&manifest).expect("serialize manifest");
    assert_eq!(json["warnings"][0], "missing symbols");
}

#[test]
fn render_profile_summary_mentions_backend_and_artifacts() {
    let manifest = sample_manifest();
    let markdown = render_profile_markdown(&manifest);
    assert!(markdown.contains("android-native"));
    assert!(markdown.contains("artifacts/raw"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mobench profile_manifest_serializes_partial_failure_state -- --exact`

Expected: FAIL because `render_profile_markdown` and the finalized manifest
shape do not exist yet.

**Step 3: Write minimal implementation**

Extend `ProfileManifest` with:

- symbolization status
- viewer hints
- capture status enum
- explicit raw/processed artifact records

Add:

```rust
pub fn render_profile_markdown(manifest: &ProfileManifest) -> String {
    // render backend, target, function, capture status, warnings, and artifact paths
}

pub fn write_profile_manifest(path: &Path, manifest: &ProfileManifest) -> Result<()> {
    // pretty JSON writer
}
```

Keep the output stable and deterministic so it is easy to diff in tests.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mobench profile_manifest_serializes_partial_failure_state -- --exact`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/mobench/src/profile.rs
git commit -m "feat: add normalized profile manifests"
```

### Task 3: Add Command Dispatch For `profile run` And `profile summarize`

**Files:**
- Modify: `crates/mobench/src/lib.rs`
- Modify: `crates/mobench/src/profile.rs`
- Test: `crates/mobench/src/lib.rs`

**Step 1: Write the failing test**

Add command-dispatch tests that verify the new commands produce expected files
in dry-run or fixture mode:

```rust
#[test]
fn profile_summarize_reads_manifest_and_prints_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let manifest_path = dir.path().join("profile.json");
    write_file(
        &manifest_path,
        serde_json::to_string_pretty(&sample_manifest()).unwrap().as_bytes(),
    )
    .unwrap();

    let output = cmd_profile_summarize_for_test(&manifest_path).expect("summarize profile");
    assert!(output.contains("sample_fns::fibonacci"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mobench profile_summarize_reads_manifest_and_prints_markdown -- --exact`

Expected: FAIL because the profile commands are not wired into execution yet.

**Step 3: Write minimal implementation**

In `crates/mobench/src/lib.rs`:

- add `Command::Profile { command: ProfileCommand }`
- dispatch to `cmd_profile_run` and `cmd_profile_summarize`
- reuse existing `write_file` and CLI output helpers

In `crates/mobench/src/profile.rs`:

- add helpers for default output paths
- add manifest loading
- add `cmd_profile_summarize_for_test`

**Step 4: Run test to verify it passes**

Run: `cargo test -p mobench profile_summarize_reads_manifest_and_prints_markdown -- --exact`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/mobench/src/lib.rs crates/mobench/src/profile.rs
git commit -m "feat: wire profile command dispatch"
```

### Task 4: Add Android Backend Command Construction With Capability Checks

**Files:**
- Modify: `crates/mobench/src/profile.rs`
- Test: `crates/mobench/src/profile.rs`

**Step 1: Write the failing test**

Add a backend-unit test that verifies Android profiling builds the expected tool
commands and errors clearly when prerequisites are missing:

```rust
#[test]
fn android_backend_requires_adb_and_simpleperf() {
    let ctx = sample_profile_context().with_backend(ProfileBackend::AndroidNative);
    let error = build_android_capture_plan(&ctx, None, None).unwrap_err();
    assert!(error.to_string().contains("simpleperf"));
}

#[test]
fn android_backend_builds_capture_plan_with_processed_artifacts() {
    let ctx = sample_profile_context().with_backend(ProfileBackend::AndroidNative);
    let plan = build_android_capture_plan(
        &ctx,
        Some(Path::new("/usr/bin/adb")),
        Some(Path::new("/opt/android/simpleperf")),
    )
    .expect("android capture plan");

    assert!(plan.raw_artifacts.iter().any(|p| p.ends_with("sample.perf")));
    assert!(plan.processed_artifacts.iter().any(|p| p.ends_with("flamegraph.html")));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mobench android_backend_requires_adb_and_simpleperf -- --exact`

Expected: FAIL because the Android profiling backend does not exist yet.

**Step 3: Write minimal implementation**

Add an Android backend that:

- detects `adb` and `simpleperf`
- allocates raw/processed artifact paths under `artifacts/raw` and
  `artifacts/processed`
- records a capture plan structure without yet attempting to optimize

Start with command construction and manifest emission only. Keep tool invocation
behind small helper functions so tests can cover planning without requiring a
real device.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mobench android_backend_requires_adb_and_simpleperf -- --exact`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/mobench/src/profile.rs
git commit -m "feat: add android profiling backend planning"
```

### Task 5: Add iOS Backend Command Construction With `xctrace`

**Files:**
- Modify: `crates/mobench/src/profile.rs`
- Test: `crates/mobench/src/profile.rs`

**Step 1: Write the failing test**

Add tests for iOS capability detection and artifact planning:

```rust
#[test]
fn ios_backend_requires_xctrace() {
    let ctx = sample_profile_context().with_backend(ProfileBackend::IosInstruments);
    let error = build_ios_capture_plan(&ctx, None).unwrap_err();
    assert!(error.to_string().contains("xctrace"));
}

#[test]
fn ios_backend_allocates_trace_bundle_and_export_paths() {
    let ctx = sample_profile_context().with_backend(ProfileBackend::IosInstruments);
    let plan = build_ios_capture_plan(&ctx, Some(Path::new("/usr/bin/xcrun")))
        .expect("ios capture plan");

    assert!(plan.raw_artifacts.iter().any(|p| p.ends_with("time-profiler.trace")));
    assert!(plan.processed_artifacts.iter().any(|p| p.ends_with("time-profiler.xml")));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mobench ios_backend_requires_xctrace -- --exact`

Expected: FAIL because the iOS backend is not implemented yet.

**Step 3: Write minimal implementation**

Add an iOS backend that:

- detects `xcrun` / `xctrace`
- builds an `xctrace record` command plan with the `Time Profiler` template
- allocates `.trace` and exported XML artifact paths
- records viewer hints that point users to Instruments

Again, keep command planning testable without requiring a live device.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mobench ios_backend_requires_xctrace -- --exact`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/mobench/src/profile.rs
git commit -m "feat: add ios profiling backend planning"
```

### Task 6: Add BrowserStack Capability Gating And Provider Errors

**Files:**
- Modify: `crates/mobench/src/profile.rs`
- Modify: `crates/mobench/src/lib.rs`
- Test: `crates/mobench/src/profile.rs`

**Step 1: Write the failing test**

Add a test that asserts BrowserStack-backed profile runs fail explicitly when a
native backend is requested:

```rust
#[test]
fn browserstack_profile_run_reports_unsupported_native_capture() {
    let ctx = sample_profile_context()
        .with_backend(ProfileBackend::AndroidNative)
        .with_provider("browserstack");

    let error = validate_profile_capabilities(&ctx).unwrap_err();
    assert!(error.to_string().contains("BrowserStack"));
    assert!(error.to_string().contains("unsupported"));
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p mobench browserstack_profile_run_reports_unsupported_native_capture -- --exact`

Expected: FAIL because capability gating is not implemented.

**Step 3: Write minimal implementation**

Add capability validation that:

- allows local Android and local iOS native backends
- rejects BrowserStack native profiling backends with a precise error
- records provider/capability warnings in dry-run and manifest output

Do not silently downgrade to another backend.

**Step 4: Run test to verify it passes**

Run: `cargo test -p mobench browserstack_profile_run_reports_unsupported_native_capture -- --exact`

Expected: PASS

**Step 5: Commit**

```bash
git add crates/mobench/src/lib.rs crates/mobench/src/profile.rs
git commit -m "feat: gate unsupported browserstack profiling backends"
```

### Task 7: Add README And Build Documentation

**Files:**
- Modify: `README.md`
- Modify: `BUILD.md`
- Modify: `TESTING.md`

**Step 1: Write the failing doc check**

Add a focused grep-based check to confirm the docs mention the new command
surface and prerequisites:

```bash
rg -n "mobench profile run|simpleperf|xctrace|profile summarize" README.md BUILD.md TESTING.md
```

Expected: no matches before the doc update.

**Step 2: Run check to verify it fails**

Run: `rg -n "mobench profile run|simpleperf|xctrace|profile summarize" README.md BUILD.md TESTING.md`

Expected: exit 1 or missing coverage for the new profiling workflow.

**Step 3: Write minimal documentation**

Document:

- the new `mobench profile` commands
- Android prerequisites (`adb`, `simpleperf`, symbols)
- iOS prerequisites (`xcrun`, `xctrace`, dSYMs)
- BrowserStack profiling limitations in the MVP

Keep the current CI v1 contract language unchanged.

**Step 4: Run check to verify it passes**

Run: `rg -n "mobench profile run|simpleperf|xctrace|profile summarize" README.md BUILD.md TESTING.md`

Expected: matching lines in all relevant docs.

**Step 5: Commit**

```bash
git add README.md BUILD.md TESTING.md
git commit -m "docs: add profiling workflow guidance"
```

### Task 8: Run Final Verification

**Files:**
- Modify: any files needed to fix verification failures

**Step 1: Run targeted tests for the new profile subsystem**

Run: `cargo test -p mobench profile_ -- --nocapture`

Expected: PASS

**Step 2: Run the full workspace test suite**

Run: `cargo test`

Expected: PASS

**Step 3: Run formatting**

Run: `cargo fmt --all --check`

Expected: PASS

**Step 4: Run a dry-run CLI smoke test**

Run:

```bash
cargo run -p mobench --bin mobench -- profile run \
  --target android \
  --function sample_fns::fibonacci \
  --backend android-native
```

Expected: PASS with a run-scoped profile session under
`target/mobench/profile/<run-id>/` and refreshed latest-session files at
`target/mobench/profile/profile.json` and `target/mobench/profile/summary.md`.

**Step 5: Commit**

```bash
git add .
git commit -m "feat: add mobench profiling subsystem"
```
