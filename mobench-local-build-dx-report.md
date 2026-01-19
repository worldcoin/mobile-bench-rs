# mobench Local Build DX Report

**Date:** 2026-01-19
**Tested Version:** Local build from `../mobile-bench-rs` (based on 0.1.11)
**Platform:** macOS Darwin 25.1.0 (arm64)

## Executive Summary

Testing the local mobench build revealed **1 new bug** (crate detection) while confirming that previous bugs from 0.1.11 remain unfixed. Both platforms build successfully, but the test APK step still fails.

**Build Status:**
- Android APK: ✅ Built successfully (133MB)
- Android Test APK: ❌ Failed (`assembleReleaseAndroidTest` task not found)
- iOS: ✅ Built successfully

---

## New Bug Found

### BUG: Crate Detection Fails Without `--crate-path`

**Severity:** CRITICAL (build won't start)

**Symptom:**
```
build error: Benchmark crate 'bench-mobile' not found.

Searched locations:
- /Users/.../bench-mobile/bench-mobile/Cargo.toml
- /Users/.../bench-mobile/crates/bench-mobile/Cargo.toml
```

**Root Cause:** mobench looks for nested directories (`bench-mobile/bench-mobile/` or `bench-mobile/crates/bench-mobile/`) instead of checking if the current directory contains a valid crate with a matching name.

**Workaround:** Use `--crate-path .` flag:
```bash
mobench build --target android --release --crate-path .
```

**Fix Required:** mobench should check if the current directory's `Cargo.toml` has a matching `[package] name`.

---

## Confirmed Bugs (Still Present from 0.1.11)

### 1. Test APK Task Not Found

**Status:** STILL PRESENT
**Impact:** Android build reports failure even though main APK builds successfully

```
Task 'assembleReleaseAndroidTest' not found in root project 'bench_mobile-android'
```

**Workaround:** Add to `app/build.gradle`:
```gradle
android {
    defaultConfig {
        testBuildType "release"
    }
}
```

---

### 2. Hardcoded local.properties

**Status:** STILL PRESENT
**File:** `target/mobench/android/local.properties`
```properties
sdk.dir=/Users/dcbuilder/Library/Android/sdk
```

**Impact:** Breaks builds on other machines.

---

### 3. Source Files Not in Package Directory

**Status:** STILL PRESENT

**Current:**
- `app/src/main/java/MainActivity.kt`
- `app/src/androidTest/java/MainActivityTest.kt`

**Expected:**
- `app/src/main/java/dev/world/bench_mobile/MainActivity.kt`
- `app/src/androidTest/java/dev/world/bench_mobile/MainActivityTest.kt`

**Impact:** Kotlin files with `package dev.world.bench_mobile` must be in matching directory structure.

---

### 4. iOS Bundle ID Duplication

**Status:** STILL PRESENT
**File:** `target/mobench/ios/BenchRunner/project.yml`

**Current:** `dev.world.benchmobile.benchmobile` (duplicated)
**Expected:** `dev.world.benchmobile`

---

### 5. Cross-Platform Naming Inconsistency

**Android:** `dev.world.bench_mobile` (snake_case)
**iOS:** `dev.world.benchmobile` (camelCase)

---

### 6. Version String Mismatch

**Android:** `0.1`
**iOS:** `1.0`

---

## Silent Failure Issues

### CRITICAL: UniFFI Cleanup Errors Swallowed

**File:** `app/src/main/java/uniffi/bench_mobile/bench_mobile.kt:895-905`
```kotlin
} catch (e: Throwable) {
    // swallow
}
```

All exceptions during `destroy()` are silently discarded, hiding memory leaks and native crashes.

---

### CRITICAL: Broad Exception Catch Without Logging

**File:** `app/src/main/java/MainActivity.kt:59-61`
```kotlin
} catch (e: Exception) {
    "Unexpected error: ${e.message}"
}
```

Catches all exceptions but doesn't log them, making production debugging impossible.

---

### HIGH: Silent Config Fallbacks

Both platforms silently fall back to defaults when config parsing fails:
- Android: `MainActivity.kt:191-197` - logs but user isn't notified
- iOS: `BenchRunnerFFI.swift:35-52` - no logging at all for invalid numeric values

---

## What's Working

✅ APK filename detection (parses `output-metadata.json`)
✅ `proguard-rules.pro` generated with correct JNA/UniFFI rules
✅ iOS bundle identifier format (no invalid characters)
✅ Gradle wrapper generated
✅ `gradle.properties` generated
✅ All template placeholders replaced
✅ x86_64 iOS simulator support (universal binary)
✅ iOS xcframework code-signed

---

## Build Outputs

### Android
```
Location: target/mobench/android/
APK: app/build/outputs/apk/release/app-release-unsigned.apk
Size: 133,561,304 bytes (127 MB)
Status: ✅ Built successfully
```

### iOS
```
Location: target/mobench/ios/
Framework: bench_mobile.xcframework/
Architectures: ios-arm64, ios-arm64_x86_64-simulator
Status: ✅ Built successfully
```

---

## Priority Fixes for Next Release

### P0 (Blocker)
1. **Fix crate detection** - Check current directory Cargo.toml, not just nested paths
2. **Fix test APK task** - Use `assembleDebugAndroidTest` or add `testBuildType` config

### P1 (Should Fix)
3. Don't generate `local.properties` with hardcoded paths
4. Fix source file directory structure (place in package path)
5. Log UniFFI cleanup errors instead of swallowing

### P2 (Nice to Have)
6. Fix iOS bundle ID duplication
7. Standardize package naming across platforms
8. Align version strings
9. Add user-visible config fallback warnings

---

## Test Commands

```bash
# Build with local mobench (requires --crate-path flag)
cd bench-mobile
mobench build --target android --release --verbose --crate-path .
mobench build --target ios --release --verbose --crate-path .

# Verify APK exists despite error
ls -la target/mobench/android/app/build/outputs/apk/release/

# Verify iOS xcframework
ls -la target/mobench/ios/bench_mobile.xcframework/
```

---

## Agent Analysis

Three debugging agents were deployed in parallel:

1. **Code Reviewer** - Confirmed test task issue, found source file directory structure bug
2. **Silent Failure Hunter** - Found 9 error handling issues (2 critical, 4 high, 3 medium)
3. **Explorer** - Found bundle ID duplication, naming inconsistencies, version mismatch

---

## Comparison: 0.1.11 vs Local Build

| Issue | 0.1.11 | Local Build |
|-------|--------|-------------|
| Crate detection | ✅ | ❌ NEW BUG |
| Test APK task | ❌ | ❌ |
| local.properties | ❌ | ❌ |
| Source file paths | ❌ | ❌ |
| iOS bundle ID dupe | ❌ | ❌ |
| proguard-rules.pro | ✅ | ✅ |
| APK detection | ✅ | ✅ |
| iOS bundle ID chars | ✅ | ✅ |
