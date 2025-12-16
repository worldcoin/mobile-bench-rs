#!/usr/bin/env bash
set -euo pipefail

# Build Rust shared libraries for Android targets using cargo-ndk.
# Prereqs (install manually in CI/local before running):
# - Android NDK and toolchains available on PATH
# - cargo-ndk installed (`cargo install cargo-ndk`)
#
# By default builds sample-fns as a cdylib, producing libsample_fns.so per ABI.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

CRATES=(${CRATES:-sample-fns})
# Add x86_64 for emulator support; keep ARM for devices.
TARGET_ABIS=("aarch64-linux-android" "armv7-linux-androideabi" "x86_64-linux-android")
API_LEVEL=24

for CRATE in "${CRATES[@]}"; do
  echo "Building Rust library for Android (crates/${CRATE})"
  for ABI in "${TARGET_ABIS[@]}"; do
    echo "  -> ${ABI}"
    cargo ndk \
      -t "${ABI}" \
      -o "${ROOT_DIR}/target/android/${ABI}" \
      --platform "${API_LEVEL}" \
      build -p "${CRATE}" --release
  done
done

echo "Finished. Outputs are under target/android/<abi>/release."
