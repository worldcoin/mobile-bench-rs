#!/usr/bin/env bash
set -euo pipefail

# Build the Rust library for iOS targets and package as xcframework.
# UniFFI-generated headers (sample_fnsFFI.h) are used for the C ABI.
#
# NOTE: If you modify the Rust API (sample_fns.udl), run:
#   cargo run --bin generate-bindings --features bindgen
# before running this script to regenerate Swift bindings and headers.
#
# Prereqs (install manually in CI/local before running):
# - Xcode command line tools
# - rustup targets: aarch64-apple-ios, aarch64-apple-ios-sim

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="sample-fns"
OUTPUT_DIR="${ROOT_DIR}/target/ios"
XCFRAMEWORK_PATH="${OUTPUT_DIR}/sample_fns.xcframework"

# iOS targets to build
IOS_TARGETS=(
  "aarch64-apple-ios"           # iOS device (ARM64)
  "aarch64-apple-ios-sim"        # iOS simulator (ARM64, M1+ Macs)
)

# Check for required iOS targets
for target in "${IOS_TARGETS[@]}"; do
  if ! rustup target list --installed | grep -q "^${target}$"; then
    echo "Installing Rust target: ${target}"
    rustup target add "${target}"
  fi
done

echo "Building Rust libraries for iOS targets"
for target in "${IOS_TARGETS[@]}"; do
  echo "  -> Building for ${target}"
  cargo build --release --target "${target}" -p "${CRATE}"
done

echo "Creating xcframework structure"
rm -rf "${XCFRAMEWORK_PATH}"
mkdir -p "${XCFRAMEWORK_PATH}"

# Create framework for each target
for target in "${IOS_TARGETS[@]}"; do
  # Static library name: lib<crate_name>.a (crate name with underscores)
  LIB_NAME="libsample_fns.a"
  LIB_PATH="${ROOT_DIR}/target/${target}/release/${LIB_NAME}"
  
  if [[ ! -f "${LIB_PATH}" ]]; then
    echo "Error: ${LIB_PATH} not found after build" >&2
    exit 1
  fi

  # Determine platform and architecture
  case "${target}" in
    aarch64-apple-ios)
      PLATFORM="ios"
      ARCH="arm64"
      FRAMEWORK_NAME="ios-arm64"
      ;;
    aarch64-apple-ios-sim)
      PLATFORM="ios-simulator"
      ARCH="arm64"
      FRAMEWORK_NAME="ios-simulator-arm64"
      ;;
    *)
      echo "Unknown target: ${target}" >&2
      exit 1
      ;;
  esac

  FRAMEWORK_DIR="${XCFRAMEWORK_PATH}/${FRAMEWORK_NAME}.framework"
  mkdir -p "${FRAMEWORK_DIR}/Headers"

  # Copy library (framework binary should match framework name)
  cp "${LIB_PATH}" "${FRAMEWORK_DIR}/${FRAMEWORK_NAME}"

  # Copy UniFFI-generated C header
  UNIFFI_HEADER="${ROOT_DIR}/ios/BenchRunner/BenchRunner/Generated/sample_fnsFFI.h"
  if [[ ! -f "${UNIFFI_HEADER}" ]]; then
    echo "Error: UniFFI header not found at ${UNIFFI_HEADER}" >&2
    echo "Run: cargo run --bin generate-bindings --features bindgen" >&2
    exit 1
  fi
  cp "${UNIFFI_HEADER}" "${FRAMEWORK_DIR}/Headers/"
  
  # Create Info.plist for this framework slice
  cat > "${FRAMEWORK_DIR}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>${FRAMEWORK_NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>dev.world.bench.sample_fns</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundlePackageType</key>
  <string>FMWK</string>
  <key>CFBundleShortVersionString</key>
  <string>1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>MinimumOSVersion</key>
  <string>13.0</string>
  <key>CFBundleSupportedPlatforms</key>
  <array>
    <string>${PLATFORM}</string>
  </array>
</dict>
</plist>
EOF

  # Create module map for UniFFI C bindings
  cat > "${FRAMEWORK_DIR}/Headers/module.modulemap" <<EOF
module sample_fns {
    header "sample_fnsFFI.h"
    export *
}
EOF
done

# Create xcframework Info.plist
cat > "${XCFRAMEWORK_PATH}/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>AvailableLibraries</key>
  <array>
    <dict>
      <key>LibraryIdentifier</key>
      <string>ios-arm64</string>
      <key>LibraryPath</key>
      <string>ios-arm64.framework</string>
      <key>SupportedArchitectures</key>
      <array>
        <string>arm64</string>
      </array>
      <key>SupportedPlatform</key>
      <string>ios</string>
    </dict>
    <dict>
      <key>LibraryIdentifier</key>
      <string>ios-simulator-arm64</string>
      <key>LibraryPath</key>
      <string>ios-simulator-arm64.framework</string>
      <key>SupportedArchitectures</key>
      <array>
        <string>arm64</string>
      </array>
      <key>SupportedPlatform</key>
      <string>ios-simulator</string>
    </dict>
  </array>
  <key>CFBundlePackageType</key>
  <string>XFWK</string>
  <key>XCFrameworkFormatVersion</key>
  <string>1.0</string>
</dict>
</plist>
EOF

echo "✓ iOS build complete. XCFramework created at: ${XCFRAMEWORK_PATH}"
