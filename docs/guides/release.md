# Release Checklist

Use this checklist before publishing the `mobench` crate family.

## Preflight

```bash
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets --all-features -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --all-features --no-deps
cargo test --workspace --locked
cargo bench -p mobench --features bench-support --bench host_contracts -- --test
```

## Publish Dry Run

Publish order matters because the SDK depends on the proc macro crate and the
CLI depends on the SDK. Before the first crate is published, only the leaf
crate can complete a crates.io dry run because unpublished sibling versions are
not yet available in the registry index.

```bash
cargo publish --dry-run -p mobench-macros
```

After `mobench-macros` is published and available from crates.io, dry-run the
SDK before publishing it:

```bash
cargo publish --dry-run -p mobench-sdk
```

After `mobench-sdk` is published and available from crates.io, dry-run the CLI:

```bash
cargo publish --dry-run -p mobench
```

## Publish

```bash
cargo publish -p mobench-macros
cargo publish -p mobench-sdk
cargo publish -p mobench
```

Wait for each crate to become available before publishing the dependent crate.
After publishing, update `RELEASE_NOTES.md` with the published date/status, push
that docs commit, then tag the release at the published commit.
