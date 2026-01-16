#!/usr/bin/env bash
set -euo pipefail

# ⚠️  DEPRECATION WARNING ⚠️
# This script is legacy tooling for developing this repository.
#
# For SDK integrators, use instead:
#   cargo mobench build --target android
#
# This command does everything this script does, but in pure Rust with no dependencies
# on having this repo's scripts/ directory locally.

# Convenience wrapper: build Rust libs for all Android ABIs, sync them into the app,
# then assemble the Android APK.
#
# Requires:
#   - cargo-ndk installed
#   - Android SDK/Gradle available

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Resolve ANDROID_NDK_HOME if not provided.
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  DEFAULT_NDK="${HOME}/Library/Android/sdk/ndk/29.0.14206865"
  if [[ -d "${DEFAULT_NDK}" ]]; then
    export ANDROID_NDK_HOME="${DEFAULT_NDK}"
    echo "ANDROID_NDK_HOME not set; defaulting to ${ANDROID_NDK_HOME}"
  else
    echo "ANDROID_NDK_HOME is not set and default NDK path not found; please export it before running." >&2
    exit 1
  fi
fi

pushd "${ROOT_DIR}" >/dev/null
./scripts/build-android.sh
ABI="${UNIFFI_ANDROID_ABI:-arm64-v8a}"
case "${ABI}" in
  arm64-v8a)
    LIB_PATH="${ROOT_DIR}/target/android/aarch64-linux-android/arm64-v8a/libsample_fns.so"
    ;;
  x86_64)
    LIB_PATH="${ROOT_DIR}/target/android/x86_64-linux-android/x86_64/libsample_fns.so"
    ;;
  armeabi-v7a)
    LIB_PATH="${ROOT_DIR}/target/android/armv7-linux-androideabi/armeabi-v7a/libsample_fns.so"
    ;;
  *)
    echo "Unknown UNIFFI_ANDROID_ABI=${ABI}; expected arm64-v8a, x86_64, or armeabi-v7a" >&2
    exit 1
    ;;
esac
UNIFFI_LIBRARY_PATH="${LIB_PATH}" ./scripts/generate-bindings.sh
./scripts/sync-android-libs.sh
popd >/dev/null

pushd "${ROOT_DIR}/android" >/dev/null
./gradlew :app:assembleDebug
popd >/dev/null

echo "Android build complete."
