# Release Guide

Current release: **0.1.49**.

Use this checklist when cutting a mobench workspace release.

## Preflight

Confirm the workspace is on the intended release commit:

```bash
git status --short
git branch --show-current
```

Run formatting and tests:

```bash
cargo fmt --all --check
cargo test --all
```

Run CLI smoke checks:

```bash
cargo run -q -p mobench --bin mobench -- --help
cargo run -q -p mobench --bin mobench -- build --help
cargo run -q -p mobench --bin mobench -- run --help
cargo run -q -p mobench --bin mobench -- ci run --help
cargo run -q -p mobench --bin mobench -- ci prepare --help
cargo run -q -p mobench --bin mobench -- ci run-prebuilt --help
cargo run -q -p mobench --bin mobench -- ci merge-split-runs --help
cargo run -q -p mobench --bin mobench -- profile run --help
```

Run documentation hygiene checks:

```bash
git diff --check
```

For a reusable-workflow security release, also run `actionlint`, the workflow
trust-boundary/self-tests, and manifest/report injection tests.

Before publication, dispatch the full release self-test from the exact
candidate branch:

```bash
gh workflow run mobile-bench-selftest.yml \
  --ref <release-candidate-branch> \
  -f platform=both \
  -f iterations=2 \
  -f warmup=1 \
  -f downstream_release_gate=true
```

The `Full mobench release gate` job must pass. It requires:

- the repository's own native fixture;
- ProveKit complete passport age-check proving on two Android and two iOS
  devices;
- World ID nullifier proving on the same native matrix;
- Rust ProveKit and World ID WASM benchmarks, plus the lockfile-pinned public
  `@worldcoin/provekit` browser SDK fixture, on macOS Safari, Windows Chrome,
  iOS Safari, and Android Chrome through BrowserStack Automate.

The downstream source revisions are immutable SHAs in
`.github/workflows/mobile-bench-selftest.yml`. Review and advance those pins
deliberately when their benchmark contracts change. Do not substitute static
workflow checks, local compilation, or an older BrowserStack run for this
service-gated release result.

Also search the docs for unfinished markers, unknown code fences, removed docs,
and old support filenames. Those searches should return no matches unless an
intentional historical note is being added.

## Versioning

All published crates should use the same release version:

- `mobench-macros`
- `mobench-sdk`
- `mobench`

Update:

- Workspace package versions.
- Internal dependency versions between the crates.
- Crate READMEs.
- Root `README.md`.
- `CHANGELOG.md`.
- `RELEASE_NOTES.md`.
- Guide release lines.

## Publish Order

Publish dependencies before dependents:

```bash
cargo publish -p mobench-macros
cargo publish -p mobench-sdk
cargo publish -p mobench
```

If crates.io indexing is still catching up, wait and retry the dependent crate.

Verify publication:

```bash
cargo search mobench --limit 5
```

## Post-Publish Checks

Install the published CLI in a clean environment:

```bash
cargo install mobench --version 0.1.49
mobench --version
mobench --help
```

Check that public docs reference the published version and do not keep
copy-pasted old-version install snippets:

```bash
rg -n '<previous-version>|mobench-sdk = "<previous-version>"' README.md docs crates -g '*.md'
rg -n '<new-version>' README.md CHANGELOG.md RELEASE_NOTES.md docs crates -g '*.md'
```

Tag the published commit:

```bash
git tag v0.1.49
git push origin v0.1.49
```

Do not add `Co-Authored-By` lines to release commits.
