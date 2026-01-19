# Mobench Developer Experience Improvements

This document captures improvements needed for mobench based on real-world integration testing with the world-id-protocol project (ZK proof benchmarking on mobile devices).

## Summary

| Priority | Task | Description | Status |
|----------|------|-------------|--------|
| P0 | [#1](#task-1-fix-aws-lc-rs-android-ndk-incompatibility) | Fix aws-lc-rs Android NDK incompatibility | DONE |
| P0 | [#2](#task-2-fix-workspace-target-directory-detection) | Fix workspace target directory detection | DONE |
| P0 | [#3](#task-3-auto-generate-project-scaffolding-during-build) | Auto-generate project scaffolding during build | DONE |
| P1 | [#4](#task-4-process-template-variables-during-build) | Process template variables during build | DONE |
| P1 | [#5](#task-5-improve-uniffi-bindgen-handling) | Improve uniffi-bindgen handling | DONE |
| P1 | [#6](#task-6-generate-error-handling-code-dynamically) | Generate error handling code dynamically | DONE |
| P2 | [#7](#task-7-add-configuration-file-support-mobenchtoml) | Add configuration file support (mobench.toml) | DONE |
| P2 | [#8](#task-8-improve-error-messages) | Improve error messages | DONE |
| P2 | [#9](#task-9-add---crate-path-flag) | Add --crate-path flag | DONE |
| P3 | [#10](#task-10-add---dry-run-and---verbose-modes) | Add --dry-run and --verbose modes | DONE |
| P3 | [#11](#task-11-auto-generate-localproperties-for-android) | Auto-generate local.properties for Android | DONE |

---

## P0 - Critical (Blocking Issues)

### Task 1: Fix aws-lc-rs Android NDK Incompatibility

**Problem**

The default `rustls` 0.23+ uses `aws-lc-rs` as the crypto backend, which fails to compile for Android NDK targets. Users see cryptic C compilation errors:

```
error occurred in cc-rs: command did not execute successfully
.../clang ... --target=aarch64-linux-android24 ... getentropy.c
```

**Root Cause**

`aws-lc-sys` contains C code that doesn't compile correctly with the Android NDK toolchain.

**Solution**

Update the generated `Cargo.toml` templates to configure rustls with the `ring` crypto backend:

```toml
[workspace.dependencies]
rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12"] }
```

**Files to Modify**

- `crates/mobench-sdk/src/codegen.rs` - Update Cargo.toml template generation
- `crates/mobench-sdk/templates/` - Update any hardcoded rustls dependencies
- `CLAUDE.md` / `BUILD.md` - Document the issue and workaround

**Acceptance Criteria**

- [ ] New projects generated with `init-sdk` compile for Android without rustls errors
- [ ] Documentation explains the aws-lc-rs issue and how to fix existing projects

---

### Task 2: Fix Workspace Target Directory Detection

**Problem**

mobench looks for the host library at `<crate_dir>/target/debug/lib*.dylib` but Cargo workspaces use a shared `<workspace_root>/target/` directory.

```
build error: host library for UniFFI not found at
"/path/to/bench-mobile/target/debug/libbench_mobile.dylib"
```

**Root Cause**

Hardcoded path assumption: `self.project_root.join("target")` instead of querying Cargo for the actual target directory.

**Solution**

Use `cargo metadata` to detect the actual target directory:

```rust
fn get_target_dir(crate_dir: &Path) -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(crate_dir)
        .output()?;
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    Ok(PathBuf::from(metadata["target_directory"].as_str().unwrap()))
}
```

**Files to Modify**

- `crates/mobench-sdk/src/builders/android.rs`
  - `generate_uniffi_bindings()` - Use correct target dir for host library
  - `copy_native_libraries()` - Use correct target dir for .so files
- `crates/mobench-sdk/src/builders/ios.rs` - Similar changes
- Consider adding `cargo_metadata` crate or manual JSON parsing

**Acceptance Criteria**

- [ ] `mobench build` works in Cargo workspace projects
- [ ] `mobench build` still works in standalone crate projects

---

### Task 3: Auto-generate Project Scaffolding During Build

**Problem**

`mobench build` assumes Android/iOS project files already exist. When missing, it fails with confusing errors:

```
build error: Failed to run Gradle: No such file or directory (os error 2)
```

Users don't know they need to run `init-sdk` first.

**Solution**

In `cmd_build()`, check if project exists and auto-generate if missing:

```rust
fn cmd_build(target: SdkTarget, ...) -> Result<()> {
    let output_dir = output_dir.unwrap_or_else(|| PathBuf::from("target/mobench"));

    if matches!(target, SdkTarget::Android | SdkTarget::Both) {
        let android_dir = output_dir.join("android");
        if !android_dir.join("build.gradle").exists() {
            println!("Android project not found, generating scaffolding...");
            generate_android_project(&android_dir, &crate_name, &template_context)?;
        }
    }

    if matches!(target, SdkTarget::Ios | SdkTarget::Both) {
        let ios_dir = output_dir.join("ios").join("BenchRunner");
        if !ios_dir.join("project.yml").exists() {
            println!("iOS project not found, generating scaffolding...");
            generate_ios_project(&ios_dir, &crate_name, &template_context)?;
        }
    }

    // Continue with normal build...
}
```

**Files to Modify**

- `crates/mobench/src/lib.rs` - Update `cmd_build()` function
- `crates/mobench-sdk/src/codegen.rs` - Extract `generate_android_project()` and `generate_ios_project()` to be callable separately from full `generate_project()`

**Acceptance Criteria**

- [ ] Running `mobench build --target android` in a fresh project generates Android scaffolding automatically
- [ ] Running `mobench build --target ios` generates iOS scaffolding automatically
- [ ] Existing projects are not overwritten

---

## P1 - High Priority

### Task 4: Process Template Variables During Build

**Problem**

Template files contain `{{VARIABLE}}` placeholders that are only processed during `init-sdk`. Users who run `build` on a project without `init-sdk` get broken files with literal `{{VAR}}` strings.

**Variables Used**

| Variable | Example Value | Description |
|----------|---------------|-------------|
| `{{PACKAGE_NAME}}` | `dev.world.bench` | Android package / iOS bundle prefix |
| `{{LIBRARY_NAME}}` | `bench_mobile` | Rust library name |
| `{{UNIFFI_NAMESPACE}}` | `bench_mobile` | UniFFI namespace (usually same as library) |
| `{{PROJECT_NAME_PASCAL}}` | `BenchRunner` | PascalCase project name |
| `{{DEFAULT_FUNCTION}}` | `my_crate::my_func` | Default benchmark function |
| `{{APP_NAME}}` | `My Bench App` | Display name |
| `{{BUNDLE_ID}}` | `dev.world.bench` | iOS bundle identifier |
| `{{BUNDLE_ID_PREFIX}}` | `dev.world` | iOS bundle prefix |

**Solution**

1. Create a `TemplateContext` struct with all variables
2. Extract template processing into a shared function
3. Call it from both `init-sdk` and `build`
4. Derive values from crate metadata when not explicitly configured

```rust
pub struct TemplateContext {
    pub package_name: String,
    pub library_name: String,
    pub uniffi_namespace: String,
    pub project_name_pascal: String,
    pub default_function: String,
    pub app_name: String,
    pub bundle_id: String,
    pub bundle_id_prefix: String,
}

impl TemplateContext {
    pub fn from_crate(crate_dir: &Path) -> Result<Self> {
        // Read Cargo.toml, extract [lib] name, derive other values
    }
}

pub fn process_templates(dir: &Path, ctx: &TemplateContext) -> Result<()> {
    for entry in WalkDir::new(dir) {
        let path = entry?.path();
        if path.is_file() {
            let content = fs::read_to_string(path)?;
            let processed = content
                .replace("{{PACKAGE_NAME}}", &ctx.package_name)
                .replace("{{LIBRARY_NAME}}", &ctx.library_name)
                // ... etc
            fs::write(path, processed)?;
        }
    }
    Ok(())
}
```

**Files to Modify**

- `crates/mobench-sdk/src/codegen.rs` - Add `TemplateContext` and `process_templates()`
- `crates/mobench-sdk/src/builders/android.rs` - Call `process_templates()` after generating scaffolding
- `crates/mobench-sdk/src/builders/ios.rs` - Same

**Acceptance Criteria**

- [ ] All `{{VAR}}` placeholders are replaced during `build`
- [ ] Values are derived from crate metadata when not configured
- [ ] Custom values can be provided via config file (see Task 7)

---

### Task 5: Improve uniffi-bindgen Handling

**Problem**

Users must manually add a `[[bin]]` target for uniffi-bindgen and install it globally. This is undocumented and confusing.

**Current Workaround (manual)**

```toml
# In bench-mobile/Cargo.toml
[[bin]]
name = "uniffi-bindgen"
path = "src/bin/uniffi-bindgen.rs"

[dependencies]
uniffi = { version = "0.28", features = ["cli"] }
```

```rust
// src/bin/uniffi-bindgen.rs
fn main() { uniffi::uniffi_bindgen_main() }
```

**Solution**

1. Generate the uniffi-bindgen binary target during project scaffolding
2. Use `cargo run -p <crate> --bin uniffi-bindgen` instead of global binary

```rust
fn run_uniffi_bindgen(crate_dir: &Path, crate_name: &str, args: &[&str]) -> Result<()> {
    // Try running via cargo first (works if crate has the binary)
    let cargo_result = Command::new("cargo")
        .args(["run", "-p", crate_name, "--bin", "uniffi-bindgen", "--"])
        .args(args)
        .current_dir(crate_dir)
        .status();

    if cargo_result.is_ok() && cargo_result.unwrap().success() {
        return Ok(());
    }

    // Fall back to global uniffi-bindgen
    let global_result = Command::new("uniffi-bindgen")
        .args(args)
        .current_dir(crate_dir)
        .status()?;

    if !global_result.success() {
        return Err(BenchError::Build(
            "uniffi-bindgen failed. Ensure your crate has a uniffi-bindgen binary \
             or install globally with: cargo install uniffi_bindgen".into()
        ));
    }

    Ok(())
}
```

**Files to Modify**

- `crates/mobench-sdk/src/codegen.rs` - Generate uniffi-bindgen binary in Cargo.toml template
- `crates/mobench-sdk/src/builders/android.rs` - Use new `run_uniffi_bindgen()` function
- `crates/mobench-sdk/src/builders/ios.rs` - Same

**Acceptance Criteria**

- [ ] New projects have uniffi-bindgen binary generated automatically
- [ ] Build works without global uniffi-bindgen installation
- [ ] Clear error message if uniffi-bindgen can't be found

---

### Task 6: Generate Error Handling Code Dynamically

**Problem**

Mobile app templates hardcode error variants like `BenchException.InvalidIterations` that may not exist in the user's UniFFI schema, causing compilation failures.

**Current Template (broken)**

```kotlin
// MainActivity.kt
} catch (e: BenchException.InvalidIterations) {
    "Error: ${e.message}"
} catch (e: BenchException.UnknownFunction) {
    "Error: ${e.message}"
```

```swift
// BenchRunnerFFI.swift
case .InvalidIterations(let message):
    return "Error: \(message)"
case .UnknownFunction(let message):
    return "Error: \(message)"
```

**Solution**

Use generic catch pattern that works with any error type:

```kotlin
// MainActivity.kt
} catch (e: BenchException) {
    "Error: ${e.message}"
} catch (e: Exception) {
    "Unexpected error: ${e.message}"
}
```

```swift
// BenchRunnerFFI.swift
private static func formatBenchError(_ error: BenchError) -> String {
    return "Error: \(error.localizedDescription)"
}
```

**Files to Modify**

- `crates/mobench-sdk/templates/android/app/src/main/java/MainActivity.kt.template`
- `crates/mobench-sdk/templates/ios/BenchRunner/BenchRunner/BenchRunnerFFI.swift.template`

**Acceptance Criteria**

- [ ] Generated Android app compiles with any BenchError variant set
- [ ] Generated iOS app compiles with any BenchError variant set
- [ ] Error messages are still descriptive

---

## P2 - Medium Priority

### Task 7: Add Configuration File Support (mobench.toml)

**Problem**

Users must pass many CLI flags repeatedly. No way to persist project configuration.

**Solution**

Support `mobench.toml` at project root:

```toml
[project]
crate = "bench-mobile"
library_name = "bench_mobile"

[android]
package = "com.example.bench"
min_sdk = 24
target_sdk = 34

[ios]
bundle_id = "com.example.bench"
deployment_target = "15.0"

[benchmarks]
default_function = "my_crate::my_benchmark"
default_iterations = 100
default_warmup = 10
```

**Files to Modify**

- `crates/mobench/src/lib.rs` - Add config file loading at startup
- `crates/mobench-sdk/src/types.rs` - Add `MobenchConfig` struct
- Create `crates/mobench/src/config.rs` module

**Acceptance Criteria**

- [ ] Config file is loaded automatically if present
- [ ] CLI flags override config file values
- [ ] `mobench init` can generate a starter config file

---

### Task 8: Improve Error Messages

**Problem**

Error messages don't explain what's expected or how to fix issues.

**Current**

```
build error: Benchmark crate 'bench-mobile' not found. Tried:
  - "/path/bench-mobile/bench-mobile"
  - "/path/bench-mobile/crates/bench-mobile"
```

**Improved**

```
build error: Benchmark crate 'bench-mobile' not found.

Searched locations:
  ✗ /path/project/bench-mobile/Cargo.toml
  ✗ /path/project/crates/bench-mobile/Cargo.toml

To fix this:
  1. Create a bench-mobile/ directory with your benchmark crate, or
  2. Use --crate-path to specify the benchmark crate location:
     mobench build --target android --crate-path ./my-benchmarks

Run 'mobench init-sdk --help' to generate a new benchmark project.
```

**Files to Modify**

- `crates/mobench-sdk/src/builders/android.rs` - All error returns
- `crates/mobench-sdk/src/builders/ios.rs` - All error returns
- `crates/mobench-sdk/src/types.rs` - Enhance `BenchError` variants with more context

**Acceptance Criteria**

- [x] All error messages include actionable fix suggestions
- [x] Searched paths are listed when file not found
- [x] Links to documentation where appropriate

---

### Task 9: Add --crate-path Flag

**Problem**

mobench hardcodes looking for `bench-mobile/` or `crates/sample-fns/`. Real projects have different structures like `crates/benchmarks/`, `benches/mobile/`, etc.

**Solution**

Add `--crate-path` flag to `build` command:

```rust
#[derive(Parser)]
struct Build {
    #[arg(long)]
    target: SdkTarget,

    /// Path to the benchmark crate (default: auto-detect bench-mobile/ or crates/sample-fns/)
    #[arg(long)]
    crate_path: Option<PathBuf>,

    #[arg(long)]
    release: bool,

    #[arg(long)]
    output_dir: Option<PathBuf>,
}
```

**Files to Modify**

- `crates/mobench/src/lib.rs` - Add CLI argument, pass to builders
- `crates/mobench-sdk/src/builders/android.rs` - Accept optional `crate_path` in constructor
- `crates/mobench-sdk/src/builders/ios.rs` - Same

**Acceptance Criteria**

- [ ] `mobench build --target android --crate-path ./my-bench` works
- [ ] Auto-detection still works when `--crate-path` not provided
- [ ] Error message suggests `--crate-path` when auto-detection fails

---

## P3 - Nice to Have

### Task 10: Add --dry-run and --verbose Modes

**Problem**

Hard to debug what mobench is doing. Users can't preview changes before they happen.

**Solution**

Add global flags:

```rust
#[derive(Parser)]
#[command(name = "mobench")]
struct Cli {
    /// Print what would be done without actually doing it
    #[arg(long, global = true)]
    dry_run: bool,

    /// Print verbose output including all commands
    #[arg(long, short = 'v', global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}
```

**Behavior**

- `--dry-run`: Print commands that would be executed, files that would be created/modified
- `--verbose`: Print all commands as they run, show full output

**Files to Modify**

- `crates/mobench/src/lib.rs` - Add global flags, thread through to all functions
- `crates/mobench-sdk/src/builders/*.rs` - Respect dry_run/verbose flags

**Acceptance Criteria**

- [ ] `mobench build --target android --dry-run` shows what would happen without making changes
- [ ] `mobench build --target android --verbose` shows all commands being run

---

### Task 11: Auto-generate local.properties for Android

**Problem**

Gradle fails without `sdk.dir` being set. Users must manually create `local.properties`:

```
SDK location not found. Define a valid SDK location with an ANDROID_HOME
environment variable or by setting the sdk.dir path in your project's
local properties file.
```

**Solution**

Auto-detect Android SDK and generate `local.properties`:

```rust
fn ensure_local_properties(android_dir: &Path) -> Result<()> {
    let local_props = android_dir.join("local.properties");
    if local_props.exists() {
        return Ok(());
    }

    let sdk_dir = detect_android_sdk()?;
    fs::write(&local_props, format!("sdk.dir={}\n", sdk_dir.display()))?;
    println!("  Generated local.properties with SDK at {}", sdk_dir.display());
    Ok(())
}

fn detect_android_sdk() -> Result<PathBuf> {
    // Check environment variables
    if let Ok(sdk) = std::env::var("ANDROID_HOME") {
        return Ok(PathBuf::from(sdk));
    }
    if let Ok(sdk) = std::env::var("ANDROID_SDK_ROOT") {
        return Ok(PathBuf::from(sdk));
    }

    // Check common locations
    let home = dirs::home_dir().ok_or_else(|| BenchError::Build("Cannot find home directory".into()))?;

    let candidates = [
        home.join("Library/Android/sdk"),      // macOS default
        home.join("Android/Sdk"),              // Linux default
        PathBuf::from("/usr/local/android-sdk"), // Common Linux location
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Ok(candidate.clone());
        }
    }

    Err(BenchError::Build(
        "Android SDK not found. Set ANDROID_HOME environment variable or install Android Studio.".into()
    ))
}
```

**Files to Modify**

- `crates/mobench-sdk/src/builders/android.rs` - Add `ensure_local_properties()`, call before Gradle

**Acceptance Criteria**

- [ ] `local.properties` is auto-generated if missing
- [ ] Existing `local.properties` is not overwritten
- [ ] Clear error if SDK cannot be found

---

## Implementation Order

Recommended order based on dependencies and impact:

1. **Task 2** - Target directory detection (unblocks workspace projects)
2. **Task 3** - Auto-generate scaffolding (simplifies getting started)
3. **Task 4** - Template processing (makes generated projects work)
4. **Task 1** - aws-lc-rs fix (unblocks Android builds)
5. **Task 6** - Error handling (makes generated code compile)
6. **Task 11** - local.properties (removes manual step)
7. **Task 5** - uniffi-bindgen (removes manual step)
8. **Task 9** - --crate-path (flexibility for real projects)
9. **Task 8** - Error messages (better debugging)
10. **Task 7** - Config file (power users)
11. **Task 10** - dry-run/verbose (debugging)

## Testing Strategy

After implementing, verify with:

1. **Fresh project test:**
   ```bash
   cargo new my-bench && cd my-bench
   cargo add mobench-sdk
   # Add a #[benchmark] function
   mobench build --target android
   # Should produce working APK
   ```

2. **Workspace project test:**
   ```bash
   git clone https://github.com/worldcoin/world-id-protocol
   cd world-id-protocol
   mobench build --target android --crate-path ./bench-mobile
   # Should produce working APK
   ```

3. **Both platforms test:**
   ```bash
   mobench build --target both
   # Should produce both APK and xcframework/IPA
   ```
