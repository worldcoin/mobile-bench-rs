#!/usr/bin/env bash
set -euo pipefail

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
./scripts/sync-android-libs.sh
popd >/dev/null

pushd "${ROOT_DIR}/android" >/dev/null
./gradlew :app:assembleDebug
popd >/dev/null

echo "Android build complete."
