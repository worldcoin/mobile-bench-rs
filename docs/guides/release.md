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
CLI depends on the SDK.

```bash
cargo publish --dry-run -p mobench-macros
cargo publish --dry-run -p mobench-sdk
cargo publish --dry-run -p mobench
```

## Publish

```bash
cargo publish -p mobench-macros
cargo publish -p mobench-sdk
cargo publish -p mobench
```

After publishing, tag the release and update `RELEASE_NOTES.md` with the
published version and any migration notes.
