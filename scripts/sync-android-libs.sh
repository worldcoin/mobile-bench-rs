#!/usr/bin/env bash
set -euo pipefail

# Copy built Rust .so files into the Android app's jniLibs structure.
# Run scripts/build-android.sh first.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_JNILIBS="${ROOT_DIR}/android/app/src/main/jniLibs"
LIB_NAME="${LIB_NAME:-sample_fns}"

declare -A ABI_MAP=(
  ["aarch64-linux-android"]="arm64-v8a"
  ["armv7-linux-androideabi"]="armeabi-v7a"
  ["x86_64-linux-android"]="x86_64"
)

for TRIPLE in "${!ABI_MAP[@]}"; do
  # Cargo NDK may place outputs under <triple>/release or directly under the ABI folder.
  SRC="${ROOT_DIR}/target/android/${TRIPLE}/release/lib${LIB_NAME}.so"
  if [[ ! -f "${SRC}" ]]; then
    ALT="${ROOT_DIR}/target/android/${TRIPLE}/${ABI_MAP[$TRIPLE]}/lib${LIB_NAME}.so"
    if [[ -f "${ALT}" ]]; then
      SRC="${ALT}"
    fi
  fi
  DEST_DIR="${APP_JNILIBS}/${ABI_MAP[$TRIPLE]}"
  if [[ ! -f "${SRC}" ]]; then
    echo "Missing ${SRC}; build first with scripts/build-android.sh" >&2
    exit 1
  fi
  mkdir -p "${DEST_DIR}"
  cp "${SRC}" "${DEST_DIR}/"
  echo "Copied ${SRC} -> ${DEST_DIR}/"
done

echo "JNI libs synced."
