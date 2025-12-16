#!/usr/bin/env bash
set -euo pipefail

# Convenience wrapper: build Rust libs for all Android ABIs, sync them into the app,
# then assemble the Android APK.
#
# Requires:
#   - ANDROID_NDK_HOME set to your NDK path (e.g. $HOME/Library/Android/sdk/ndk/29.0.14206865)
#   - cargo-ndk installed
#   - Android SDK/Gradle available

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  echo "ANDROID_NDK_HOME is not set; please export it before running." >&2
  exit 1
fi

pushd "${ROOT_DIR}" >/dev/null
./scripts/build-android.sh
./scripts/sync-android-libs.sh
popd >/dev/null

pushd "${ROOT_DIR}/android" >/dev/null
./gradlew :app:assembleDebug
popd >/dev/null

echo "Android build complete."
