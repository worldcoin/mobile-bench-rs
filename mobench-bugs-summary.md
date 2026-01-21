# mobench Bug Summary for Development

**Last Updated:** 2026-01-19
**Tested Versions:** 0.1.9, 0.1.10, 0.1.11, Local Build
**Test Crate:** bench-mobile (World ID ZK Proof Benchmarks)

---

## Quick Status

| Version | Critical Bugs | Builds Without Fixes | Recommendation |
|---------|---------------|---------------------|----------------|
| 0.1.9 | 12 | ❌ No | Do not use |
| 0.1.10 | 2 | ✅ Yes (false failure) | Usable |
| 0.1.11 | 1 | ⚠️ Partial | Usable with workaround |
| Local | 2 | ⚠️ Requires --crate-path | Needs crate detection fix |

---

## Version Evolution

### 0.1.9 → 0.1.10 (12 bugs fixed)
All template placeholder bugs fixed, Gradle wrapper and properties added.

### 0.1.10 → 0.1.11 (3 bugs fixed, 1 new bug)

**Fixed:**
- ✅ APK filename detection (now parses output-metadata.json)
- ✅ iOS bundle identifier (removed invalid hyphens/underscores)
- ✅ proguard-rules.pro now generated

**New Bug:**
- ❌ `assembleReleaseAndroidTest` task not found

### 0.1.11 → Local Build (0 bugs fixed, 1 new bug)

**New Bug:**
- ❌ Crate detection fails - requires `--crate-path .` flag

**Still Present:**
- ❌ `assembleReleaseAndroidTest` task not found
- ❌ Hardcoded local.properties
- ❌ Source files not in package directory
- ❌ iOS bundle ID duplication

---

## Current Bugs in Local Build

### CRITICAL

#### Bug 0: Crate Detection Fails (NEW in Local Build)
**Location:** mobench-sdk crate detection logic
**Symptom:**
```
build error: Benchmark crate 'bench-mobile' not found.

Searched locations:
- /path/bench-mobile/bench-mobile/Cargo.toml
- /path/bench-mobile/crates/bench-mobile/Cargo.toml
```

**Cause:** mobench looks for nested directories instead of checking if the current directory is a valid crate.

**Impact:**
- Build fails to start without workaround
- Must use `--crate-path .` flag

**Workaround:**
```bash
mobench build --target android --release --crate-path .
```

---

#### Bug 1: Test APK Task Not Found (from 0.1.11)
**Location:** mobench-sdk android.rs
**Symptom:**
```
Task 'assembleReleaseAndroidTest' not found in root project
```

**Cause:** Android Gradle only creates test tasks for debug by default. Release test task requires `testBuildType "release"`.

**Impact:**
- Main APK builds ✅
- Test APK fails ❌
- mobench reports overall failure

**Workaround:**
```gradle
// Add to app/build.gradle
android {
    defaultConfig {
        testBuildType "release"
    }
}
```

**Or:** Use debug build type for tests (recommended)

---

### HIGH

#### Bug 2: Hardcoded local.properties (Still present)
**File:** `target/mobench/android/local.properties`
```properties
sdk.dir=/Users/dcbuilder/Library/Android/sdk
```

**Impact:** Breaks builds on other machines

**Fix:** Don't generate this file

---

#### Bug 3: Test File Wrong Directory
**Current:** `app/src/androidTest/java/MainActivityTest.kt`
**Expected:** `app/src/androidTest/java/dev/world/bench_mobile/MainActivityTest.kt`

**Impact:** Test compilation may fail

---

### MEDIUM

#### Bug 4: iOS Bundle ID Duplication
**Current:** `dev.world.benchmobile.benchmobile`
**Expected:** `dev.world.benchmobile.BenchRunner`

**Impact:** Cosmetic, works but non-standard

---

#### Bug 5: Silent Error Fallbacks
Both platforms catch exceptions broadly and lose error context.

---

## Fixed Bugs (Historical)

### Fixed in 0.1.11
| Bug | Description | Status |
|-----|-------------|--------|
| APK filename | Expected wrong name | ✅ Fixed |
| iOS bundle ID | Invalid chars | ✅ Fixed |
| proguard-rules.pro | Missing file | ✅ Fixed |

### Fixed in 0.1.10
| Bug | Description | Status |
|-----|-------------|--------|
| `{{PACKAGE_NAME}}` | Not replaced | ✅ Fixed |
| `{{LIBRARY_NAME}}` | Not replaced | ✅ Fixed |
| `{{PROJECT_NAME}}` | Not replaced | ✅ Fixed |
| `{{APP_NAME}}` | Not replaced | ✅ Fixed |
| gradle.properties | Missing | ✅ Fixed |
| Gradle wrapper | Missing | ✅ Fixed |
| AGP version | Invalid 8.13.2 | ✅ Fixed |
| x86_64 iOS sim | Missing | ✅ Fixed |

---

## Verification Commands

```bash
# Build Android (APK succeeds, test APK fails)
mobench build --target android --release --verbose
ls -la target/mobench/android/app/build/outputs/apk/release/

# Build iOS (succeeds fully)
mobench build --target ios --release --verbose
ls -la target/mobench/ios/bench_mobile.xcframework/

# Workaround for test APK
cd target/mobench/android
# Add testBuildType to build.gradle, then:
./gradlew assembleReleaseAndroidTest
```

---

## Recommended Priority Fixes for Next Release

### P0 (Blocker)
1. **Fix crate detection** - Check current directory Cargo.toml, not just nested paths
2. **Fix test APK task** - Use debug or add testBuildType config

### P1 (Should Fix)
3. Don't generate local.properties with hardcoded paths
4. Fix source file directory structure (place in package path)
5. Log UniFFI cleanup errors instead of swallowing

### P2 (Nice to Have)
6. Fix iOS bundle ID duplication
7. Standardize package naming across platforms
8. Improve error handling to preserve types

---

## Files Changed

- `bench-mobile/Cargo.toml` - Using local mobench-sdk path
- `docs/mobench-0.1.9-dx-report.md` - Full 0.1.9 report
- `docs/mobench-0.1.10-dx-report.md` - Full 0.1.10 report
- `docs/mobench-0.1.11-dx-report.md` - Full 0.1.11 report
- `docs/mobench-local-build-dx-report.md` - Full local build report
- `docs/mobench-bugs-summary.md` - This file
