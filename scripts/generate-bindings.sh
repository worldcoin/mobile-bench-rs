#!/usr/bin/env bash
set -euo pipefail

# Generate Kotlin and Swift bindings using UniFFI

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="${ROOT_DIR}/crates/sample-fns"

# Build the library for host first
echo "Building sample-fns..."
cargo build -p sample-fns

# Determine library extension based on platform
if [[ "$OSTYPE" == "darwin"* ]]; then
    LIB_EXT="dylib"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    LIB_EXT="so"
else
    echo "Unsupported platform: $OSTYPE"
    exit 1
fi

LIB_PATH="${ROOT_DIR}/target/debug/libsample_fns.${LIB_EXT}"

# Generate Kotlin bindings
echo "Generating Kotlin bindings..."
cargo run --bin uniffi-bindgen generate \
  --library "${LIB_PATH}" \
  --language kotlin \
  --out-dir "${ROOT_DIR}/android/app/src/main/java"

# Generate Swift bindings
echo "Generating Swift bindings..."
mkdir -p "${ROOT_DIR}/ios/BenchRunner/BenchRunner/Generated"
cargo run --bin uniffi-bindgen generate \
  --library "${LIB_PATH}" \
  --language swift \
  --out-dir "${ROOT_DIR}/ios/BenchRunner/BenchRunner/Generated"

echo "✓ Bindings generated successfully"
echo "  - Kotlin: android/app/src/main/java/uniffi/sample_fns/"
echo "  - Swift: ios/BenchRunner/BenchRunner/Generated/"
