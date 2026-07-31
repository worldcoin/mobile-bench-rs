#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <provekit-checkout> <mobench-checkout>" >&2
  exit 2
fi

provekit_root="$(cd "$1" && pwd)"
mobench_root="$(cd "$2" && pwd)"
fixture_root="${mobench_root}/tests/release-fixtures/provekit-wasm"
manifest="${provekit_root}/Cargo.toml"

PROVEKIT_ROOT="$provekit_root" MOBENCH_ROOT="$mobench_root" python3 - <<'PY'
import os
from pathlib import Path

provekit = Path(os.environ["PROVEKIT_ROOT"])
mobench = Path(os.environ["MOBENCH_ROOT"])
root_manifest = provekit / "Cargo.toml"
bench_manifest = provekit / "bench-mobile/Cargo.toml"

root_text = root_manifest.read_text()
old_sdk = 'mobench-sdk = { version = "0.1.47", default-features = false, features = ["registry"] }'
new_sdk = (
    f'mobench-sdk = {{ path = "{mobench / "crates/mobench-sdk"}", '
    'default-features = false, features = ["registry"] }'
)
if root_text.count(old_sdk) != 1:
    raise SystemExit("unexpected ProveKit mobench-sdk dependency")
root_manifest.write_text(root_text.replace(old_sdk, new_sdk))
root_text = root_manifest.read_text()
old_prover = 'provekit-prover = { path = "provekit/prover", version = "1.0.0" }'
new_prover = (
    'provekit-prover = { path = "provekit/prover", version = "1.0.0", '
    'default-features = false }'
)
if root_text.count(old_prover) != 1:
    raise SystemExit("unexpected ProveKit prover workspace dependency")
root_text = root_text.replace(old_prover, new_prover)
for member in (
    "tooling/cli",
    "tooling/provekit-bench",
    "tooling/provekit-ffi",
    "tooling/provekit-gnark",
    "tooling/provekit-wasm",
    "tooling/verifier-server",
):
    entry = f'  "{member}",\n'
    if root_text.count(entry) != 1:
        raise SystemExit(f"unexpected ProveKit workspace member: {member}")
    root_text = root_text.replace(entry, "")
root_manifest.write_text(root_text)

bench_text = bench_manifest.read_text()
anchor = "anyhow.workspace = true\n"
if bench_text.count(anchor) != 1:
    raise SystemExit("unexpected ProveKit bench-mobile dependency layout")
bench_text = bench_text.replace(
    anchor,
    anchor + "acir.workspace = true\npostcard.workspace = true\n",
)
old_prover = "provekit-prover.workspace = true\n"
if bench_text.count(old_prover) != 1:
    raise SystemExit("unexpected ProveKit bench-mobile prover dependency")
bench_text = bench_text.replace(old_prover, "")
bench_text += (
    '\n[target.\'cfg(not(target_arch = "wasm32"))\'.dependencies]\n'
    'provekit-prover = { workspace = true, features = ["witness-generation", "parallel"] }\n'
    '\n[target.\'cfg(target_arch = "wasm32")\'.dependencies]\n'
    'provekit-prover.workspace = true\n'
)
example = '\n[[example]]\nname = "export-complete-age-check-witness"\npath = "examples/export_complete_age_check_witness.rs"\n'
bench_manifest.write_text(bench_text + example)
PY

js_sys_version="$(
  MOBENCH_ROOT="$mobench_root" python3 - <<'PY'
import os, tomllib
from pathlib import Path

lock = tomllib.loads((Path(os.environ["MOBENCH_ROOT"]) / "Cargo.lock").read_text())
print(next(package["version"] for package in lock["package"] if package["name"] == "js-sys"))
PY
)"
cargo update --manifest-path "$manifest" -p js-sys --precise "$js_sys_version"

mkdir -p "${provekit_root}/bench-mobile/examples"
cp "${fixture_root}/export_witness.rs" \
  "${provekit_root}/bench-mobile/examples/export_complete_age_check_witness.rs"

(
  cd "$provekit_root"
  for attempt in 1 2 3; do
    if MOBENCH_CI_PREPARE=1 ./bench-mobile/scripts/generate-fixtures.sh; then
      break
    fi
    if [ "$attempt" -eq 3 ]; then
      echo "ProveKit fixture generation failed after 3 attempts" >&2
      exit 1
    fi
    echo "ProveKit fixture generation attempt ${attempt} failed; retrying" >&2
    sleep "$((attempt * 5))"
  done
  cargo run --release -p bench-mobile --example export-complete-age-check-witness -- \
    noir-examples/noir-passport-monolithic/complete_age_check/target/complete_age_check.json \
    noir-examples/noir-passport-monolithic/complete_age_check/Prover.toml \
    bench-mobile/generated/complete_age_check.witness.postcard
)

cp "${fixture_root}/lib.rs" "${provekit_root}/bench-mobile/src/lib_web.rs"

PROVEKIT_ROOT="$provekit_root" python3 - <<'PY'
import os
from pathlib import Path

manifest = Path(os.environ["PROVEKIT_ROOT"]) / "bench-mobile/Cargo.toml"
text = manifest.read_text()
old = '[lib]\ncrate-type = ["lib", "cdylib", "staticlib"]'
new = '[lib]\npath = "src/lib_web.rs"\ncrate-type = ["lib", "cdylib", "staticlib"]'
if text.count(old) != 1:
    raise SystemExit("unexpected ProveKit bench-mobile lib target")
manifest.write_text(text.replace(old, new))
PY
