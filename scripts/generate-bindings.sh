#!/usr/bin/env bash
set -euo pipefail

# ⚠️  DEPRECATION WARNING ⚠️
# This script is legacy tooling for developing this repository.
#
# For SDK integrators, bindings are automatically generated during:
#   cargo mobench build --target <android|ios>
#
# You don't need to call this script separately.

# Generate Kotlin and Swift bindings using UniFFI

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="${ROOT_DIR}/crates/sample-fns"

if [[ -n "${UNIFFI_LIBRARY_PATH:-}" ]]; then
  echo "Using UNIFFI_LIBRARY_PATH=${UNIFFI_LIBRARY_PATH}"
else
  echo "Building sample-fns (release)..."
  cargo build -p sample-fns --release
  export UNIFFI_PROFILE=release
fi

echo "Generating Kotlin + Swift bindings via sample-fns helper..."
cargo run -p sample-fns --bin generate-bindings --features bindgen
