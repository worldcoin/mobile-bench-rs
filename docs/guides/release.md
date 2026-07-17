# Release Guide

Current release candidate: **0.1.45**.

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
trust-boundary/self-tests, and manifest/report injection tests. Before
publication, record one Android and one iOS BrowserStack benchmark through
`ci run-prebuilt`; do not substitute static workflow checks for these
service-gated runs.

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
cargo install mobench --version 0.1.45
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
git tag v0.1.45
git push origin v0.1.45
```

Do not add `Co-Authored-By` lines to release commits.
