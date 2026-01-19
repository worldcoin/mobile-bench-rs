# mobench 0.1.11 DX (Developer Experience) Report

**Date:** 2026-01-19
**Tested Version:** mobench-sdk 0.1.11, mobench CLI 0.1.11
**Platform:** macOS Darwin 25.1.0 (arm64)
**Previous Versions Tested:** 0.1.9, 0.1.10

## Executive Summary

mobench 0.1.11 fixes 3 major issues from 0.1.10 but introduces 1 new critical bug. The APK build succeeds, but the new test APK build step fails because it uses a non-existent Gradle task.

**Build Status:**
- Android APK: ✅ Built successfully (133MB)
- Android Test APK: ❌ Failed (`assembleReleaseAndroidTest` task not found)
- iOS: ✅ Built successfully

---

## Version Comparison Summary

| Issue | 0.1.9 | 0.1.10 | 0.1.11 |
|-------|-------|--------|--------|
| Template placeholders | ❌ 5 unfilled | ✅ Fixed | ✅ Fixed |
| Gradle wrapper | ❌ Missing | ✅ Generated | ✅ Generated |
| gradle.properties | ❌ Missing | ✅ Generated | ✅ Generated |
| AGP version | ❌ Invalid | ✅ Fixed | ✅ Fixed |
| x86_64 iOS simulator | ❌ Missing | ✅ Included | ✅ Included |
| APK filename detection | N/A | ❌ Wrong name | ✅ Parses metadata |
| iOS bundle ID chars | N/A | ❌ Invalid | ✅ Fixed |
| proguard-rules.pro | ❌ Missing | ❌ Missing | ✅ Generated |
| Test APK build | N/A | N/A | ❌ **NEW BUG** |

---

## Improvements in 0.1.11 (Fixed from 0.1.10)

### 1. APK Filename Detection - FIXED
**Previous:** Expected `app-release.apk`, build reported failure
**Now:** Parses `output-metadata.json` to find correct filename `app-release-unsigned.apk`

### 2. iOS Bundle Identifier - FIXED
**Previous:** `dev.world.bench-mobile.bench_mobile` (invalid chars)
**Now:** `dev.world.benchmobile.benchmobile` (valid)

### 3. ProGuard Rules File - FIXED
**Previous:** File missing, ProGuard would fail if enabled
**Now:** `proguard-rules.pro` generated with proper JNA/UniFFI keep rules

### 4. iOS Config Logging - IMPROVED
**Previous:** Silent fallback to defaults
**Now:** Logs warnings when config file missing or invalid

---

## New Bug in 0.1.11

### CRITICAL: `assembleReleaseAndroidTest` Task Not Found

**Symptom:**
```
Task 'assembleReleaseAndroidTest' not found in root project 'bench_mobile-android'
```

**Root Cause:**
mobench 0.1.11 now attempts to build a test APK after the main APK. It uses `assembleReleaseAndroidTest` for release builds, but this Gradle task doesn't exist unless explicitly configured.

**Why:** Android Gradle Plugin only creates test APK tasks for the debug build type by default. The release test task requires `testBuildType "release"` in `build.gradle`.

**Impact:**
- Main APK builds successfully (133MB)
- mobench reports overall failure due to test APK step
- BrowserStack Espresso tests cannot run against release builds

**Fix Required in mobench-sdk:**
Either:
1. Always use `assembleDebugAndroidTest` (test APKs are debug anyway)
2. Add `testBuildType "release"` to generated `build.gradle`
3. Make test APK build optional

**Workaround:**
Add to `app/build.gradle`:
```gradle
android {
    defaultConfig {
        testBuildType "release"
    }
}
```

---

## Remaining Issues (Not Fixed)

### HIGH: Hardcoded local.properties
**File:** `target/mobench/android/local.properties`
**Issue:** Contains machine-specific SDK path
```properties
sdk.dir=/Users/dcbuilder/Library/Android/sdk
```
**Impact:** Breaks builds on other machines

### MEDIUM: Bundle ID Duplication (iOS)
**Current:** `dev.world.benchmobile.benchmobile`
**Expected:** `dev.world.benchmobile.BenchRunner`
**Impact:** Cosmetic, doesn't break builds

### MEDIUM: Test File Wrong Directory (Android)
**Current:** `app/src/androidTest/java/MainActivityTest.kt`
**Expected:** `app/src/androidTest/java/dev/world/bench_mobile/MainActivityTest.kt`
**Impact:** Test may not compile correctly

### MEDIUM: Silent Error Fallbacks
Both Android and iOS catch exceptions broadly and lose error type information.

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

## Agent Analysis Summary

Three debugging agents were deployed:

1. **Android Reviewer** - Found test task configuration issue, confirmed proguard-rules.pro fix
2. **iOS Reviewer** - Confirmed bundle ID fix, found duplication issue
3. **Silent Failure Hunter** - Found error handling patterns, test directory structure issue

All agents identified the `assembleReleaseAndroidTest` as the critical new bug.

---

## Test Commands

```bash
# Upgrade
cargo install mobench --version 0.1.11 --force

# Update SDK
# In bench-mobile/Cargo.toml: mobench-sdk = "0.1.11"
cargo update -p mobench-sdk

# Clean and build
rm -rf target/mobench
mobench build --target android --release --verbose  # Fails at test APK
mobench build --target ios --release --verbose      # Succeeds

# Verify APK exists despite error
ls -la target/mobench/android/app/build/outputs/apk/release/

# Workaround: Add testBuildType to build.gradle and retry
echo 'android { defaultConfig { testBuildType "release" } }' >> target/mobench/android/app/build.gradle
cd target/mobench/android && ./gradlew assembleReleaseAndroidTest
```

---

## Priority Fixes for 0.1.12

### P0 (Blocker)
1. **Fix test APK task** - Use `assembleDebugAndroidTest` or add `testBuildType` config

### P1 (High)
2. **Don't generate local.properties** with hardcoded paths
3. **Fix test file directory** - Place in correct package directory

### P2 (Medium)
4. Fix iOS bundle ID duplication
5. Improve error handling to preserve error types

---

## Overall Score

| Version | Score | Builds Without Fixes |
|---------|-------|---------------------|
| 0.1.9 | 4/10 | ❌ No |
| 0.1.10 | 8/10 | ✅ Yes (false failure) |
| 0.1.11 | 7/10 | ⚠️ Partial (APK yes, test APK no) |

**Note:** 0.1.11 would be 9/10 if the test APK task issue is fixed. The other improvements (APK detection, proguard, iOS bundle ID) are significant.

---

## Conclusion

mobench 0.1.11 makes good progress on DX issues but introduces a regression with the test APK build. The main APK builds successfully and is usable. For production use, either:
1. Ignore the test APK failure if not using BrowserStack Espresso
2. Apply the workaround to add `testBuildType "release"`
3. Wait for 0.1.12 with the fix
