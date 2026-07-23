#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: $0 <world-id-protocol-checkout> <mobench-checkout>" >&2
  exit 2
fi

world_id_root="$(cd "$1" && pwd)"
mobench_root="$(cd "$2" && pwd)"

WORLD_ID_ROOT="$world_id_root" MOBENCH_ROOT="$mobench_root" python3 - <<'PY'
import os
from pathlib import Path

root = Path(os.environ["WORLD_ID_ROOT"])
mobench = Path(os.environ["MOBENCH_ROOT"])
manifest = root / "crates/zk-mobile-bench/Cargo.toml"
source = root / "crates/zk-mobile-bench/src/lib.rs"
web_source = root / "crates/zk-mobile-bench/src/lib_web.rs"

text = manifest.read_text()
replacements = {
    '[lib]\ncrate-type = ["lib", "cdylib", "staticlib"]':
        '[lib]\npath = "src/lib_web.rs"\ncrate-type = ["lib", "cdylib", "staticlib"]',
    'mobench-sdk = "0.1.40"':
        f'mobench-sdk = {{ path = "{mobench / "crates/mobench-sdk"}" }}',
    'world-id-proof = { workspace = true }':
        'world-id-proof = { workspace = true, features = ["embed-zkeys"] }',
}
for old, new in replacements.items():
    if text.count(old) != 1:
        raise SystemExit(f"unexpected world-id-protocol fixture layout: {old}")
    text = text.replace(old, new)

for dependency in (
    'world-id-core = { workspace = true, default-features = false, features = ["authenticator", "embed-zkeys", "issuer"] }\n',
    'uniffi = { version = "0.31", features = ["cli"] }\n',
):
    if text.count(dependency) != 1:
        raise SystemExit(f"unexpected world-id-protocol dependency: {dependency.strip()}")
    text = text.replace(dependency, "")
manifest.write_text(text)

source_text = source.read_text()
marker = "// UniFFI Exports for Mobile"
if source_text.count(marker) != 1:
    raise SystemExit("unexpected world-id-protocol UniFFI marker")
prefix = source_text.split(marker, 1)[0]
web_source.write_text(prefix)
PY
