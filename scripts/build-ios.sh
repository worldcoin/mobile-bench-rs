#!/usr/bin/env bash
set -euo pipefail

# Build the Rust library for iOS targets and generate a header for the C ABI.
# Prereqs (install manually in CI/local before running):
# - Xcode command line tools
# - rustup targets for aarch64-apple-ios and aarch64-apple-ios-sim
# - cargo-apple (recommended) or cargo-lipo
# - cbindgen (`cargo install cbindgen`)

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE="sample-fns"
OUTPUT_DIR="${ROOT_DIR}/target/ios"
HEADER_DIR="${OUTPUT_DIR}/include"

mkdir -p "${HEADER_DIR}"
echo "Generating C header via cbindgen"
cbindgen "${ROOT_DIR}/crates/${CRATE}" \
  --config "${ROOT_DIR}/crates/${CRATE}/cbindgen.toml" \
  --output "${HEADER_DIR}/sample_fns.h"

echo "Building xcframework for ${CRATE}"
cargo apple build --release --platform ios --target-dir "${OUTPUT_DIR}" -p "${CRATE}" || {
  echo "cargo-apple not installed or failed; install via 'cargo install cargo-apple' and retry."
  exit 1
}

echo "Finished. Outputs are under ${OUTPUT_DIR}. If using the iOS app harness, point the Xcode project dependency at the generated xcframework."
