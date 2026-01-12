# Build Reference Guide

Complete build instructions for Android and iOS targets.

## Table of Contents
- [Prerequisites](#prerequisites)
- [Android Build](#android-build)
- [iOS Build](#ios-build)
- [Common Issues](#common-issues)

## Prerequisites

### All Platforms
```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify installation
rustc --version
cargo --version
```

### Android
```bash
# Install Android NDK via Android Studio or sdkmanager
# Set environment variable (add to ~/.zshrc or ~/.bashrc)
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/29.0.14206865

# Install required Rust targets
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

# Install cargo-ndk
cargo install cargo-ndk

# Verify NDK installation
ls $ANDROID_NDK_HOME
```

### iOS (macOS only)
```bash
# Install Xcode from App Store

# Install command-line tools
xcode-select --install

# Install xcodegen
brew install xcodegen

# Install required Rust targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# Verify installation
xcodegen --version
xcodebuild -version
```

## Android Build

### Quick Start (Recommended)
```bash
# Build everything and create APK in one command
./scripts/build-android-app.sh

# Install on connected device or emulator
adb install -r android/app/build/outputs/apk/debug/app-debug.apk

# Launch the app
adb shell am start -n dev.world.bench/.MainActivity
```

### Step-by-Step Build

#### Step 1: Build Rust Libraries
```bash
./scripts/build-android.sh
```

This compiles Rust code for three Android ABIs:
- `aarch64-linux-android` → `arm64-v8a` (64-bit ARM devices)
- `armv7-linux-androideabi` → `armeabi-v7a` (32-bit ARM devices)
- `x86_64-linux-android` → `x86_64` (x86 emulators)

Output: `target/android/{abi}/release/libsample_fns.so`

#### Step 2: Sync Libraries to Android Project
```bash
./scripts/sync-android-libs.sh
```

This copies `.so` files to `android/app/src/main/jniLibs/{abi}/libsample_fns.so` where Android's build system expects them.

#### Step 3: Build APK with Gradle
```bash
cd android
./gradlew :app:assembleDebug
cd ..
```

Output: `android/app/build/outputs/apk/debug/app-debug.apk`

#### Step 4: Install and Run
```bash
# Install
adb install -r android/app/build/outputs/apk/debug/app-debug.apk

# Launch with default parameters
adb shell am start -n dev.world.bench/.MainActivity

# Or launch with custom benchmark parameters
adb shell am start -n dev.world.bench/.MainActivity \
  --es bench_function sample_fns::checksum \
  --ei bench_iterations 50 \
  --ei bench_warmup 10
```

### Using Android Studio
1. Build Rust libraries first:
   ```bash
   ./scripts/build-android.sh
   ./scripts/sync-android-libs.sh
   ```

2. Open the `android/` directory in Android Studio

3. Wait for Gradle sync to complete

4. Click Run (green play button) or Run → Run 'app'

5. Select target device/emulator

### Rebuild After Code Changes
```bash
# If Rust code changed
./scripts/build-android.sh
./scripts/sync-android-libs.sh

# If only Kotlin/Java changed
cd android && ./gradlew :app:assembleDebug

# Full clean rebuild
cargo clean
./scripts/build-android-app.sh
```

## iOS Build

### Quick Start (Recommended)
```bash
# Build Rust xcframework (includes automatic code signing)
./scripts/build-ios.sh

# Generate Xcode project
cd ios/BenchRunner
xcodegen generate

# Open in Xcode
open BenchRunner.xcodeproj
```

Then in Xcode:
- Select a simulator (e.g., iPhone 15) from the device menu
- Click Run (⌘+R)

### Step-by-Step Build

#### Step 1: Build Rust XCFramework
```bash
./scripts/build-ios.sh
```

This script:
1. Compiles Rust for iOS targets:
   - `aarch64-apple-ios` (physical devices)
   - `aarch64-apple-ios-sim` (M1+ Mac simulators)

2. Creates xcframework with structure:
   ```
   target/ios/sample_fns.xcframework/
   ├── Info.plist
   ├── ios-arm64/
   │   └── sample_fns.framework/
   │       ├── sample_fns (static library)
   │       ├── Headers/
   │       │   ├── sample_fnsFFI.h
   │       │   └── module.modulemap
   │       └── Info.plist
   └── ios-simulator-arm64/
       └── sample_fns.framework/
           ├── sample_fns (static library)
           ├── Headers/
           │   ├── sample_fnsFFI.h
           │   └── module.modulemap
           └── Info.plist
   ```

3. Copies UniFFI-generated C headers into each framework slice

4. Creates module maps for Swift interoperability

5. **Automatically code-signs the xcframework** (required for Xcode)

Output: `target/ios/sample_fns.xcframework` (signed)

**Note**: The build script now includes automatic code signing. If signing fails for any reason, you can sign manually:
```bash
codesign --force --deep --sign - target/ios/sample_fns.xcframework
```

Code signing is **required** for Xcode to accept and link the framework. Without signing, you'll see "The Framework 'sample_fns.xcframework' is unsigned" errors.

#### Step 2: Generate Xcode Project
```bash
cd ios/BenchRunner
xcodegen generate
```

This generates `BenchRunner.xcodeproj` from `project.yml` specification. The generated project includes:
- Source files from `BenchRunner/` directory
- Generated Swift bindings (`BenchRunner/Generated/sample_fns.swift`)
- Bridging header (`BenchRunner/BenchRunner-Bridging-Header.h`)
- Framework dependency on `../../target/ios/sample_fns.xcframework`

#### Step 3: Build and Run in Xcode
```bash
open BenchRunner.xcodeproj
```

In Xcode:
1. Select scheme: **BenchRunner**
2. Select destination: **iPhone 15** (or any simulator, or physical device)
3. Click Run (⌘+R) or Product → Run

The app will launch and display benchmark results.

### Custom Benchmark Parameters

#### Method 1: Environment Variables in Xcode
1. Product → Scheme → Edit Scheme...
2. Run → Arguments → Environment Variables
3. Add variables:
   - `BENCH_FUNCTION` = `sample_fns::checksum`
   - `BENCH_ITERATIONS` = `50`
   - `BENCH_WARMUP` = `10`
4. Close and run

#### Method 2: Command Line (Simulator)
```bash
# Build for simulator
xcodebuild -project ios/BenchRunner/BenchRunner.xcodeproj \
  -scheme BenchRunner \
  -destination 'platform=iOS Simulator,name=iPhone 15' \
  -derivedDataPath ios/build

# Launch with arguments
xcrun simctl launch booted dev.world.bench \
  --bench-function=sample_fns::checksum \
  --bench-iterations=50 \
  --bench-warmup=10
```

### Rebuild After Code Changes
```bash
# If Rust code changed (includes automatic signing)
./scripts/build-ios.sh

# If Swift code changed, just rebuild in Xcode (⌘+B)

# If project.yml changed
cd ios/BenchRunner
xcodegen generate
open BenchRunner.xcodeproj

# Full clean rebuild
cargo clean
./scripts/build-ios.sh
cd ios/BenchRunner
xcodegen generate
# Clean in Xcode (⌘+Shift+K) then build (⌘+B)
```

### Important iOS Notes

**Static Frameworks**: The xcframework contains static libraries (`.a` files), not dynamic frameworks. This means:
- The framework is linked at compile time
- No module import is needed in Swift (`import sample_fns` is NOT used)
- A bridging header exposes C FFI types to Swift
- The UniFFI-generated Swift bindings are compiled directly into the app

**Bridging Header**: The project uses `BenchRunner-Bridging-Header.h` to import the C FFI:
```objc
#import "sample_fnsFFI.h"
```

This makes C types (`RustBuffer`, `RustCallStatus`, etc.) available to Swift without explicit imports.

**Code Signing**: The build script (`build-ios.sh`) automatically signs the xcframework. If you build manually or signing fails, sign with:
```bash
codesign --force --deep --sign - target/ios/sample_fns.xcframework
```

## Common Issues

### Android

**Issue**: `ANDROID_NDK_HOME is not set`
```bash
# Find your NDK installation
find ~/Library/Android/sdk/ndk -name "ndk-build" 2>/dev/null

# Export the path (add to ~/.zshrc or ~/.bashrc)
export ANDROID_NDK_HOME=$HOME/Library/Android/sdk/ndk/29.0.14206865
```

**Issue**: `cargo-ndk: command not found`
```bash
cargo install cargo-ndk
```

**Issue**: App crashes with `UnsatisfiedLinkError`
```bash
# Ensure .so files are in the APK
./scripts/sync-android-libs.sh
cd android && ./gradlew clean assembleDebug

# Verify .so files are in APK
unzip -l android/app/build/outputs/apk/debug/app-debug.apk | grep libsample_fns.so
```

**Issue**: `Error: UnknownFunction`
- Check function name is correct: `fibonacci`, `checksum`, `sample_fns::fibonacci`, `sample_fns::checksum`
- Function names are case-sensitive

### iOS

**Issue**: `xcodegen: command not found`
```bash
brew install xcodegen
```

**Issue**: "The Framework 'sample_fns.xcframework' is unsigned"
```bash
codesign --force --deep --sign - target/ios/sample_fns.xcframework
```

**Issue**: "While building for iOS Simulator, no library for this platform was found"
```bash
# Rebuild with correct structure
rm -rf target/ios/sample_fns.xcframework
./scripts/build-ios.sh
codesign --force --deep --sign - target/ios/sample_fns.xcframework

# Clean Xcode build
cd ios/BenchRunner
xcodebuild clean -project BenchRunner.xcodeproj -scheme BenchRunner
```

**Issue**: "Unable to find module dependency: 'sample_fns'"
- Remove any `import sample_fns` statements from Swift code
- The types are available globally via the bridging header

**Issue**: "Cannot find type 'RustBuffer' in scope"
```bash
# Ensure bridging header exists
cat ios/BenchRunner/BenchRunner/BenchRunner-Bridging-Header.h

# Should contain:
# #import "sample_fnsFFI.h"

# Regenerate project
cd ios/BenchRunner
xcodegen generate
```

**Issue**: "framework 'ios-simulator-arm64' not found"
- The framework binary or directory structure is incorrect
- Rebuild: `./scripts/build-ios.sh`
- Verify structure: Each framework should be named `sample_fns.framework`, not the platform identifier

**Issue**: "Framework had an invalid CFBundleIdentifier"
- Framework bundle ID conflicts with app bundle ID
- Check `scripts/build-ios.sh` uses `dev.world.sample-fns` for framework
- App uses `dev.world.bench`

## UniFFI Bindings

If you modify the Rust API (`crates/sample-fns/src/sample_fns.udl`):

```bash
# Regenerate bindings
cargo run --bin generate-bindings --features bindgen

# This updates:
# - android/app/src/main/java/uniffi/sample_fns/sample_fns.kt (Kotlin)
# - ios/BenchRunner/BenchRunner/Generated/sample_fns.swift (Swift)
# - ios/BenchRunner/BenchRunner/Generated/sample_fnsFFI.h (C header)

# Then rebuild mobile apps
./scripts/build-android-app.sh  # Android
./scripts/build-ios.sh          # iOS
codesign --force --deep --sign - target/ios/sample_fns.xcframework
```

## Performance Testing

Run benchmarks locally without mobile builds:
```bash
cargo run -p bench-cli -- demo --iterations 100 --warmup 10
```

## Additional Documentation

- **`TESTING.md`**: Comprehensive testing guide with troubleshooting
- **`README.md`**: Project overview and quick start
- **`CLAUDE.md`**: Developer guide for working with this codebase
- **`PROJECT_PLAN.md`**: Architecture and roadmap
