# Codebase Concerns

**Analysis Date:** 2026-01-21

## Tech Debt

**Large Monolithic Build Modules:**
- Issue: iOS builder (`crates/mobench-sdk/src/builders/ios.rs`, 1938 lines) and Android builder (`crates/mobench-sdk/src/builders/android.rs`, 1346 lines) are large and complex. The iOS builder handles project scaffolding, Rust compilation, binding generation, xcframework creation, code signing, IPA packaging, and XCUITest packaging all in one module.
- Files: `crates/mobench-sdk/src/builders/ios.rs`, `crates/mobench-sdk/src/builders/android.rs`
- Impact: Difficult to maintain, test, and modify individual build steps. Changes to one aspect can unexpectedly affect others. Errors in one phase block subsequent phases with no ability to resume.
- Fix approach: Refactor each builder into smaller, composable build stages (separate modules for compilation, binding generation, packaging). Consider a builder pattern or pipeline architecture where each stage is independent and can be tested/executed separately.

**Unwrap Calls in Production Code:**
- Issue: Multiple `.unwrap()` calls in production code paths that panic on failure instead of returning errors properly.
- Files: `crates/mobench-sdk/src/builders/ios.rs` lines 1401, 1409, 1555, 1609, 1619, 1726, 1727 (in `package_ipa()` and `package_xcuitest()` methods)
- Impact: Panics on non-UTF8 paths (rare but possible on non-ASCII filesystems) instead of returning proper `BenchError`. Makes error messages unhelpful and breaks build pipelines.
- Fix approach: Replace `.to_str().unwrap()` with `.to_str().ok_or_else(|| BenchError::Build("Path contains non-UTF8 characters".to_string()))` throughout the builders.

**Path Canonicalization Without Error Handling:**
- Issue: `IosBuilder::new()` calls `canonicalize().unwrap_or(root_input)` which silently falls back to non-canonical paths if canonicalization fails (`crates/mobench-sdk/src/builders/ios.rs:135`).
- Files: `crates/mobench-sdk/src/builders/ios.rs:135`
- Impact: Can cause subtle path resolution issues, especially with symlinks or relative paths. The fallback hides the failure and can lead to incorrect artifact locations or permission errors later in the build.
- Fix approach: Propagate canonicalization errors to the caller, or add logging to explicitly warn when fallback is used. Consider making canonicalization optional based on path type.

**Simple String Parsing for Cargo Metadata:**
- Issue: `common::get_cargo_target_dir()` uses manual string parsing (`find()` and string slicing) instead of JSON parsing to extract the target directory from `cargo metadata` output (`crates/mobench-sdk/src/builders/common.rs:129-136`).
- Files: `crates/mobench-sdk/src/builders/common.rs:129-136`
- Impact: Brittle - fragile to any changes in cargo metadata format. Parsing fails silently with fallback to workspace detection which may be wrong. Unescaped Windows paths with backslashes are manually handled.
- Fix approach: Use `serde_json` to parse the cargo metadata JSON output properly, eliminating the need for manual string parsing.

**Generated Code Not Validated Before Use:**
- Issue: Template files are rendered and written to disk but never validated. Generated Kotlin, Swift, and Gradle files are not checked for syntax errors or basic correctness before being used in builds.
- Files: `crates/mobench-sdk/src/codegen.rs` (template rendering functions), `crates/mobench-sdk/src/builders/android.rs` (build step), `crates/mobench-sdk/src/builders/ios.rs` (build step)
- Impact: Invalid templates or rendering errors can cause builds to fail deep in Gradle/Xcode with cryptic compiler errors far from the source. No early warning if template variables are malformed.
- Fix approach: Add a validation step after template rendering to check file syntax or at least verify that critical files are readable and non-empty before passing to downstream tools.

## Known Bugs

**iOS XCUITest Packaging Fails With Complex Paths:**
- Symptoms: `cargo mobench package-xcuitest` command fails with "zip: invalid path" errors when the project path contains spaces or special characters.
- Files: `crates/mobench-sdk/src/builders/ios.rs:1726` (zip command construction)
- Trigger: Run `cargo mobench package-xcuitest` in a directory with spaces in the path (e.g., `/Users/name/My Projects/bench`)
- Workaround: Place the project in a path without spaces and relative path separators.

**Cargo Metadata Fallback Silently Fails in Workspaces:**
- Symptoms: Users building in a Cargo workspace get builds that reference the wrong target directory, leading to "library not found" errors.
- Files: `crates/mobench-sdk/src/builders/common.rs:110-122` (fallback logic)
- Trigger: Run `cargo mobench build` from a nested crate within a workspace where the target directory is at the workspace root.
- Workaround: Use explicit `--crate-path` flag to specify the crate directory, or run from the workspace root.

**Template Variable Name Collision in Generated Code:**
- Symptoms: If a user's project or crate name is "sample_fns", template variable substitution might collide with the default example crate name.
- Files: `crates/mobench-sdk/src/codegen.rs` (template rendering with `render_template()`)
- Trigger: Create a new project named "sample_fns" with `cargo mobench init`.
- Workaround: Rename project to avoid collision with reserved example crate names.

## Security Considerations

**BrowserStack Credentials Logged in Verbose Output:**
- Risk: When using `--verbose` flag, build commands containing BrowserStack credentials in environment variables or config files may appear in output sent to logs or CI systems.
- Files: `crates/mobench/src/lib.rs` (verbose printing), `crates/mobench-sdk/src/builders/common.rs:225-247` (run_command)
- Current mitigation: Credentials are only passed in HTTP headers or environment, not on command lines. Dotenvy loads from `.env.local` but this is separate.
- Recommendations: Filter credential values from verbose command output before printing. Add a note in docs warning users not to share verbose build logs if credentials are in the environment. Consider adding a `--no-log-env` flag to explicitly exclude sensitive env vars from output.

**File Paths From User Input Not Validated:**
- Risk: User-provided paths in `--crate-path`, `--output-dir`, `--ios-app`, `--ios-test-suite` are used directly in `fs::` operations and command construction without path traversal validation.
- Files: `crates/mobench/src/lib.rs` (CLI argument parsing), `crates/mobench-sdk/src/builders/ios.rs`, `crates/mobench-sdk/src/builders/android.rs`
- Current mitigation: Paths are treated as absolute or relative to cwd; no symlink following or special escaping.
- Recommendations: Validate that user-provided paths don't escape the expected project root (e.g., using `std::path::Path::canonicalize()` and checking that the canonical path is within the project). Document path handling behavior clearly.

**ZIP and Command Execution With Unchecked Output:**
- Risk: `zip` and `unzip` commands are executed with paths from `fs::` operations. While paths are not shell-injected (using `Command::arg()` not shell), very long paths or paths with control characters could cause issues.
- Files: `crates/mobench-sdk/src/builders/ios.rs:1555`, `1726` (zip command)
- Current mitigation: Using `Command` struct prevents shell injection. Paths are from filesystem operations so control characters are unlikely.
- Recommendations: Add length validation for paths before passing to external tools. Consider using a pure Rust ZIP library instead of external `zip` command to avoid subprocess overhead and path escaping issues.

**Credentials in Configuration Files Not Sanitized:**
- Risk: BrowserStack credentials stored in `bench-config.toml` or environment variables can leak in error messages or debug output.
- Files: `crates/mobench/src/config.rs`, `crates/mobench/src/lib.rs` (credential handling)
- Current mitigation: Credentials are stored as strings; if they end up in error context they'll be visible.
- Recommendations: Implement a `Secret<T>` wrapper type that redacts values in `Display` and `Debug` implementations. Use this for all credential fields.

## Performance Bottlenecks

**Sequential Rust Compilation for Multiple Targets:**
- Problem: iOS and Android builders compile for multiple targets sequentially (iOS: `aarch64-apple-ios` + `aarch64-apple-ios-sim` + optionally `x86_64-apple-ios`; Android: `aarch64-linux-android` + `armv7-linux-androideabi` + `x86_64-linux-android`). Each `cargo build --target X` is a full rebuild.
- Files: `crates/mobench-sdk/src/builders/ios.rs:480-536` (build_rust_libraries), `crates/mobench-sdk/src/builders/android.rs` (similar)
- Cause: Sequential `Command::output()` calls for each target; no parallelization or incremental caching.
- Improvement path: Use `rayon` or `parking_lot` to compile targets in parallel. Implement caching of intermediate artifacts to skip unchanged targets. Consider `cargo build -p crate --target-dir=X` for separate artifact caching.

**Full Cargo Metadata Parsing on Every Build:**
- Problem: Every build invokes `cargo metadata --format-version 1 --no-deps` which scans the workspace and serializes JSON. This happens even if the target directory hasn't changed.
- Files: `crates/mobench-sdk/src/builders/common.rs:94-148` (get_cargo_target_dir)
- Cause: Called on every builder invocation without caching.
- Improvement path: Cache the metadata result per project root with a file-system mtime check on Cargo.lock/Cargo.toml. Invalidate cache if those files change.

**Xcframework Creation With Manual Directory Manipulation:**
- Problem: xcframework creation involves creating many directories, copying files, and writing plist files individually. This is slow and error-prone.
- Files: `crates/mobench-sdk/src/builders/ios.rs:714-800` (create_xcframework)
- Cause: Iterating over targets and manually constructing directory structures.
- Improvement path: Batch directory operations, use parallel copying for large framework sizes, or use a library for xcframework creation.

## Fragile Areas

**XCFramework Info.plist Construction:**
- Files: `crates/mobench-sdk/src/builders/ios.rs:714-800` (create_xcframework, Info.plist generation)
- Why fragile: Info.plist structure is manually constructed as a string with hardcoded XML. Any formatting error breaks Xcode's parsing. The structure is complex with nested arrays and dictionaries.
- Safe modification: Use a plist library (e.g., `plist` crate) instead of string formatting. Add unit tests that validate the generated plist can be parsed by Apple's tools. Document the exact Info.plist schema required for each iOS version.
- Test coverage: No integration tests for Info.plist generation. Only static structure is tested; actual Xcode compatibility is not verified in CI.

**Template Rendering With Variable Substitution:**
- Files: `crates/mobench-sdk/src/codegen.rs:580-640` (render_dir and render_template functions)
- Why fragile: Simple string replacement (`str::replace()`) without escaping or validation. If a template variable contains the delimiter sequence, it can cause double-substitution or broken output. No safeguards against incomplete substitution.
- Safe modification: Use a proper templating engine (e.g., `tera`, `askama`) that handles escaping and validates all variables are substituted. Add a post-rendering validation step that checks for unreplaced variables (e.g., `{USER_CRATE}` appearing in final output).
- Test coverage: Template tests exist but only for basic cases. No tests for special characters in variable values or edge cases like empty strings or very long names.

**Gradle Build Configuration:**
- Files: `crates/mobench-sdk/templates/android/app/build.gradle.kts` (embedded template), `crates/mobench-sdk/src/builders/android.rs:650-750` (Gradle invocation)
- Why fragile: Gradle build is invoked with minimal error handling. If Gradle version, Android SDK, or NDK changes, the build can silently succeed but produce incorrect artifacts. No validation of artifact existence after build completes.
- Safe modification: Add post-build artifact validation (check APK/test APK exist and are valid ZIP files). Run `gradle --version` and `sdkmanager --list` at build start to validate environment. Add stricter error handling for Gradle command failures.
- Test coverage: No tests for Gradle integration; only Rust compilation is tested in isolation.

**CLI Argument Parsing and Validation:**
- Files: `crates/mobench/src/lib.rs:137-270` (CLI struct and command enum)
- Why fragile: Many optional arguments with interdependencies (e.g., `--ios-app` requires `--target ios`, `--fetch` requires `--output`). These constraints are validated deep in business logic, not in the arg parser. No mutual exclusivity rules in clap.
- Safe modification: Add clap validators and group rules to enforce constraints at parse time. Use `#[command(subcommand_required = true)]` to ensure a command is selected. Add unit tests for arg parsing edge cases.
- Test coverage: Only happy-path argument combinations are tested. Error cases for invalid argument combinations are not covered.

## Scaling Limits

**APK Size Limits With BrowserStack Uploads:**
- Current capacity: Debug APK ~544MB, Release APK ~133MB. BrowserStack uploads have implicit timeouts (~10-30 minutes depending on network).
- Limit: APK size over ~200MB can cause upload timeouts or failures. Complex applications with large dependencies hit this quickly.
- Scaling path: Reduce APK size by enabling ProGuard/R8 minification, using modularization (dynamic features), or stripping unnecessary native libraries. The `--release` flag helps but is not sufficient for large codebases. Consider shipping just the benchmark runner app without unnecessary dependencies.

**Number of Benchmark Functions:**
- Current capacity: The registry system (using `inventory` crate) can discover 100s of functions, but there's no limit testing.
- Limit: Unknown; potential issue with very large registries (1000+) causing slow startup or memory issues.
- Scaling path: Profile registry initialization under load. Consider lazy registration or dynamic function discovery instead of compile-time registration.

**Device List Size in Parallel Runs:**
- Current capacity: CLI accepts multiple devices via `--devices` flag. Each device gets its own BrowserStack session.
- Limit: No limit enforced on CLI, but BrowserStack API has rate limits and concurrent session limits (typically 5-10 per account).
- Scaling path: Add validation to warn users if device count exceeds account limits. Implement queueing or automatic parallelization with backoff for large device matrices.

## Dependencies at Risk

**Rustls Crypto Backend Selection:**
- Risk: The code uses `rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12"] }` to explicitly use the `ring` backend instead of `aws-lc-rs` (the default). This is fragile because it depends on rustls version and feature evolution.
- Impact: If a transitive dependency switches to `rustls` without the `ring` feature, Android builds will fail with unresolved symbols or linking errors.
- Migration plan: Monitor rustls releases for changes to default crypto backend. Consider vendoring rustls configuration in workspace.dependencies. Alternatively, wait for `aws-lc-rs` to support Android NDK targets properly.

**Uniffi Version Lock:**
- Risk: Code uses `uniffi = "0.28"` (workspace-managed). Uniffi has frequent updates with breaking changes. Version mismatch between macro crate and runtime can cause build failures.
- Impact: If a user pins `mobench-macros` to 0.1.13 but their project uses a different `uniffi` version, FFI code generation and binding compilation will fail.
- Migration plan: Lock uniffi version consistently across all workspace members. Add a CI check to verify version consistency. Document the exact uniffi version supported by each mobench release.

**Include-Dir Embedded Templates:**
- Risk: Templates are embedded at compile time using `include_dir!()`. Any change to templates requires recompilation of `mobench-sdk`. There's no way to update templates without a new release.
- Impact: Bugs in generated project templates (e.g., incorrect Gradle config) can't be fixed for users with old CLI versions.
- Migration plan: Consider distributing templates as separate files loaded from a well-known location (e.g., `~/.mobench/templates/`) with fallback to embedded. This allows users to fix broken templates without CLI updates.

## Missing Critical Features

**Resume on Partial Build Failure:**
- Problem: If a build fails halfway through (e.g., APK builds but IPA doesn't), the entire build is lost. There's no checkpoint/resume mechanism.
- Blocks: Users building for both platforms can't easily retry just the failed platform without rebuilding the successful one.

**Test Coverage for Mobile Artifacts:**
- Problem: Built APK/IPA files are never validated for correctness (no check for required classes, entry points, or artifact integrity).
- Blocks: Users can't detect build issues until they try to run on a real device.

**Incremental Rebuilds:**
- Problem: Every `cargo mobench build` recompiles all Rust targets and regenerates all bindings, even if nothing changed.
- Blocks: Development iteration is slow; users can't quickly test changes to just one component.

**Android Version Compatibility Matrix:**
- Problem: Code supports arbitrary min_sdk and target_sdk values but doesn't validate them or provide guidance on compatibility.
- Blocks: Users may select combinations that produce non-functional APKs without warning.

## Test Coverage Gaps

**Builder Error Paths Not Tested:**
- What's not tested: Scenarios where `cargo build` fails, `xcodebuild` fails, or external tools are missing are not covered. Only happy-path builds are tested.
- Files: `crates/mobench-sdk/src/builders/android.rs` (tests end at line ~1350), `crates/mobench-sdk/src/builders/ios.rs` (tests end at line ~1936)
- Risk: Error handling code is untested and may panic or produce unhelpful messages when tools fail. Users will struggle to diagnose build issues.
- Priority: High

**Template Generation Edge Cases:**
- What's not tested: Template rendering with special characters in project names, very long names, or non-ASCII characters. Package name sanitization is not tested.
- Files: `crates/mobench-sdk/src/codegen.rs` (render functions, sanitization functions)
- Risk: Users with special project names may get invalid Android/iOS project structures that fail downstream.
- Priority: Medium

**BrowserStack Integration End-to-End:**
- What's not tested: Actual BrowserStack API communication beyond unit tests. No integration tests with a real BrowserStack account.
- Files: `crates/mobench/src/browserstack.rs` (client implementation)
- Risk: Subtle API changes or version differences may break in production only.
- Priority: Medium

**CLI Argument Validation:**
- What's not tested: Invalid argument combinations (e.g., `--local-only` with `--devices`), missing required arguments in config, mismatched platform-specific args.
- Files: `crates/mobench/src/lib.rs` (CLI command validation logic)
- Risk: Users get confusing error messages or unexpected behavior from invalid argument combinations.
- Priority: Medium

**Cross-Platform Path Handling:**
- What's not tested: Windows paths with non-ASCII characters, UNC paths, symlinks, or relative paths in various scenarios.
- Files: `crates/mobench-sdk/src/builders/common.rs` (path operations throughout)
- Risk: Builds fail on Windows or with unusual filesystem configurations.
- Priority: Low (if Windows support is not a current goal)

---

*Concerns audit: 2026-01-21*
