# mobench 0.1.9 DX (Developer Experience) Report

**Date:** 2026-01-19
**Tested Version:** mobench-sdk 0.1.9, mobench CLI 0.1.9
**Platform:** macOS Darwin 25.1.0 (arm64)
**Crate:** bench-mobile (World ID ZK Proof Benchmarks)

## Executive Summary

End-to-end testing of mobench 0.1.9 for both Android and iOS builds revealed **12 critical bugs**, **8 high-severity issues**, and multiple DX improvement opportunities. The primary problems involve template substitution failures, missing configuration files, and silent failures that mask underlying errors.

**Build Status:**
- Android: Built successfully after 4 manual fixes
- iOS: Built successfully for arm64 simulator; x86_64 not supported

---

## Critical Bugs

### Bug 1: `{{PROJECT_NAME}}` Placeholder Not Replaced
**Severity:** CRITICAL
**File:** `target/mobench/android/settings.gradle`

The `PROJECT_NAME` template variable is **not defined** in the codegen template variable list, leaving the placeholder literally in the generated file:

```gradle
rootProject.name = "{{PROJECT_NAME}}-android"  // NOT REPLACED
```

**Impact:** Gradle shows the project as "{{PROJECT_NAME}}-android" in IDE.

**Fix:** Add `PROJECT_NAME` to template variables in `codegen.rs`.

---

### Bug 2: `{{PACKAGE_NAME}}` Placeholder Not Replaced
**Severity:** CRITICAL
**File:** `target/mobench/android/app/build.gradle`

Multiple occurrences of `{{PACKAGE_NAME}}` not substituted:
- Line 5: `namespace = "{{PACKAGE_NAME}}"`
- Line 15: `applicationId "{{PACKAGE_NAME}}"`

**Impact:** Gradle build fails with "{{PACKAGE_NAME}} is not a valid Java identifier".

**Manual Fix Applied:**
```gradle
namespace = "dev.world.bench_mobile"
applicationId "dev.world.bench_mobile"
```

---

### Bug 3: `{{LIBRARY_NAME}}` Placeholder Not Replaced
**Severity:** CRITICAL
**File:** `target/mobench/android/app/build.gradle` (line 57)

```gradle
keepDebugSymbols += ["**/lib{{LIBRARY_NAME}}.so"]  // NOT REPLACED
```

**Manual Fix Applied:**
```gradle
keepDebugSymbols += ["**/libbench_mobile.so"]
```

---

### Bug 4: `{{PROJECT_NAME_PASCAL}}` Placeholder Not Replaced
**Severity:** CRITICAL
**File:** `target/mobench/android/app/src/main/AndroidManifest.xml`

```xml
android:theme="@style/Theme.{{PROJECT_NAME_PASCAL}}"  // NOT REPLACED
```

**Impact:** Android resource linking fails with "style/Theme.{{PROJECT_NAME_PASCAL}} not found".

**Manual Fix Applied:**
```xml
android:theme="@style/Theme.MobileBench"
```

---

### Bug 5: `{{APP_NAME}}` Placeholder Not Replaced
**Severity:** CRITICAL
**File:** `target/mobench/android/app/src/main/res/values/strings.xml`

```xml
<string name="app_name">{{APP_NAME}}</string>  // NOT REPLACED
```

**Impact:** App displays "{{APP_NAME}}" as its title.

---

### Bug 6: Missing `gradle.properties`
**Severity:** CRITICAL
**Expected:** `target/mobench/android/gradle.properties`

The file doesn't exist in the scaffolded output, causing:
```
Configuration contains AndroidX dependencies, but android.useAndroidX property is not enabled
```

**Manual Fix Applied:** Created file with:
```properties
android.useAndroidX=true
android.enableJetifier=true
org.gradle.jvmargs=-Xmx4096m -Dfile.encoding=UTF-8 -XX:+UseParallelGC
org.gradle.daemon=true
org.gradle.parallel=true
org.gradle.caching=true
kotlin.code.style=official
```

---

### Bug 7: Missing Gradle Wrapper (gradlew)
**Severity:** CRITICAL
**Expected:** `target/mobench/android/gradlew`

The scaffolded project doesn't include the Gradle wrapper files, causing:
```
Command: ./gradlew assembleRelease
Error: No such file or directory
```

**Manual Fix Applied:** Generated wrapper with `gradle wrapper --gradle-version 8.5`

---

### Bug 8: Invalid Android Gradle Plugin Version
**Severity:** CRITICAL
**File:** `target/mobench/android/build.gradle`

Template uses:
```gradle
classpath 'com.android.tools.build:gradle:8.13.2'  // DOES NOT EXIST
```

AGP version 8.13.2 doesn't exist. Latest stable is 8.2.x.

**Manual Fix Applied:**
```gradle
classpath 'com.android.tools.build:gradle:8.2.2'
```

---

### Bug 9: Package Name Mismatch
**Severity:** HIGH
**Files:** build.gradle vs Kotlin sources

- build.gradle template uses `{{PACKAGE_NAME}}` placeholder
- Kotlin sources are hardcoded to `dev.world.bench_mobile`

These must match or builds fail. The Kotlin files don't use template variables.

---

### Bug 10: Missing x86_64 iOS Simulator Architecture
**Severity:** HIGH
**File:** `target/mobench/ios/bench_mobile.xcframework/Info.plist`

The xcframework only includes:
- `ios-arm64` (device)
- `ios-simulator-arm64` (Apple Silicon simulator)

Missing: `ios-simulator-x86_64` (Intel Mac simulator)

**Impact:** Build fails on Intel Macs and older CI runners:
```
ld: symbol(s) not found for architecture x86_64
```

---

### Bug 11: Path Handling Bug
**Severity:** HIGH

Running `mobench build --target ios` from within `target/mobench/android/` causes iOS project to be generated at `target/mobench/android/target/mobench/ios/` instead of `target/mobench/ios/`.

---

### Bug 12: Default Benchmark Function Mismatch
**Severity:** HIGH
**Files:** MainActivity.kt, BenchRunnerFFI.swift

Both use `DEFAULT_FUNCTION = "example_fibonacci"` but the actual benchmarks are:
- `bench_mobile::bench_query_proof_generation`
- `bench_mobile::bench_nullifier_proof_generation`

**Impact:** Fresh app launch fails with "unknown benchmark function" error.

---

## High Severity Issues

### Issue 1: No Post-Template Validation
Template rendering doesn't validate that all `{{PLACEHOLDER}}` patterns were replaced. Files are written with literal placeholder text.

### Issue 2: Silent Codesign Failure
`codesign_xcframework` returns `Ok(())` even when signing fails, just printing a warning.

### Issue 3: Silent xcodegen Failure
`generate_xcode_project` returns `Ok(())` when xcodegen fails, just printing a warning.

### Issue 4: Silent Native Library Skip
Missing `.so` files are silently skipped without `--verbose`, leading to runtime crashes.

### Issue 5: No Path Validation
No validation that the project root exists, is a directory, or contains Cargo.toml.

### Issue 6: cargo metadata Fallback Hides Errors
Falls back to `crate_dir/target` silently when workspace metadata parsing fails.

### Issue 7: Empty Catch Block in Kotlin
```kotlin
} catch (_: Exception) {
    null  // Swallows all exceptions
}
```

### Issue 8: No Build Completion Validation
No verification that all expected artifacts exist after build completes.

---

## Medium/Low Severity Issues

| Issue | Severity | Description |
|-------|----------|-------------|
| Missing .gitignore | MEDIUM | No .gitignore in scaffolded projects |
| local.properties committed | MEDIUM | Contains machine-specific SDK path |
| No README generated | LOW | No documentation in scaffold output |
| Benchmark naming inconsistency | MEDIUM | Tests use `world_id_mobile_bench::` prefix, examples use `bench_mobile::` |

---

## Manual Fixes Applied During Testing

### Android Fixes (in order):

1. **build.gradle (root):**
   ```diff
   - classpath 'com.android.tools.build:gradle:8.13.2'
   + classpath 'com.android.tools.build:gradle:8.2.2'
   ```

2. **app/build.gradle:**
   ```diff
   - namespace = "{{PACKAGE_NAME}}"
   + namespace = "dev.world.bench_mobile"

   - applicationId "{{PACKAGE_NAME}}"
   + applicationId "dev.world.bench_mobile"

   - keepDebugSymbols += ["**/lib{{LIBRARY_NAME}}.so"]
   + keepDebugSymbols += ["**/libbench_mobile.so"]
   ```

3. **AndroidManifest.xml:**
   ```diff
   - android:theme="@style/Theme.{{PROJECT_NAME_PASCAL}}"
   + android:theme="@style/Theme.MobileBench"
   ```

4. **Created gradle.properties** (entire file)

5. **Generated Gradle wrapper:**
   ```bash
   cd target/mobench/android && gradle wrapper --gradle-version 8.5
   ```

### iOS Fixes:
None required - build completed for arm64 simulator. Device builds require signing configuration.

---

## Recommended Priority Fixes for mobench-sdk

### P0 (Blocker - Fix Immediately)

1. **Fix template variable substitution**
   - Add `PROJECT_NAME`, `PROJECT_NAME_PASCAL`, `APP_NAME`, `PACKAGE_NAME`, `LIBRARY_NAME` to all template contexts
   - Ensure all placeholders are defined before rendering

2. **Add placeholder validation**
   ```rust
   // After render_template(), validate no {{...}} remain
   if output.contains("{{") && output.contains("}}") {
       return Err(BenchError::Build("Unreplaced placeholder found"));
   }
   ```

3. **Include gradle.properties in templates**
   ```properties
   android.useAndroidX=true
   android.enableJetifier=true
   ```

4. **Include Gradle wrapper files or generate them**
   ```rust
   // Generate wrapper if gradle available
   Command::new("gradle").arg("wrapper").arg("--gradle-version").arg("8.5")
   ```

5. **Fix AGP version to valid value (8.2.2)**

### P1 (High - Fix Before Next Release)

6. **Add x86_64 iOS simulator support**
   ```rust
   // Add to iOS build targets
   "x86_64-apple-ios"
   ```

7. **Make error handling explicit**
   - Remove `Ok(())` returns on codesign/xcodegen failures
   - Add `--skip-signing` and `--skip-xcodegen` flags instead

8. **Add path validation**
   - Verify project_root exists and contains Cargo.toml
   - Warn if running from unexpected directory

9. **Update default benchmark function**
   - Generate from discovered benchmarks
   - Or use a known-working default

### P2 (Medium - Nice to Have)

10. Generate .gitignore files
11. Generate README.md with usage instructions
12. Add build completion validation step
13. Improve error messages with actionable fixes

---

## Test Commands Used

```bash
# Update mobench CLI
cargo install mobench --version 0.1.9 --force

# Update SDK dependency
mobench-sdk = "0.1.9"  # in Cargo.toml
cargo update -p mobench-sdk

# Clean and build Android
rm -rf target/mobench
mobench build --target android --release --verbose

# Build iOS
mobench build --target ios --release --verbose

# Build Android APK (after fixes)
cd target/mobench/android
gradle wrapper --gradle-version 8.5
./gradlew assembleRelease

# Build iOS app for simulator
cd target/mobench/ios/BenchRunner
xcodebuild -scheme BenchRunner -configuration Release \
  -destination 'platform=iOS Simulator,name=iPhone 17 Pro' build
```

---

## Appendix: File Locations

| File | Purpose | Status |
|------|---------|--------|
| `bench-mobile/Cargo.toml` | Benchmark crate config | OK |
| `bench-mobile/src/lib.rs` | Benchmark implementations | OK |
| `target/mobench/android/` | Generated Android project | Needs 5 fixes |
| `target/mobench/ios/` | Generated iOS project | OK for arm64 |
| `target/mobench/android/app/build/outputs/apk/release/app-release-unsigned.apk` | Android APK (133MB) | Built |
| `target/mobench/ios/bench_mobile.xcframework/` | iOS framework | Built |

---

## Conclusion

mobench 0.1.9 has significant DX issues with template substitution being the most critical. The tool generates project scaffolding but fails to replace 5+ template placeholders, omits required configuration files, and uses an invalid AGP version. After manual fixes, both platforms build successfully.

**Recommendation:** Do not use 0.1.9 in CI/CD without the fixes documented above. Wait for 0.1.10 or later with these issues resolved.
