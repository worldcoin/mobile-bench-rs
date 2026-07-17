# Fetch Results Guide

Current release: **0.1.45**.

Use `--fetch` or `cargo mobench fetch` to collect BrowserStack artifacts after a
remote benchmark run. Fetching is for benchmark logs and session artifacts;
native profile artifacts are local-only.

## Fetch During A Run

```bash
cargo mobench run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --fetch-output-dir target/browserstack \
  --output target/mobench/results.json
```

For CI:

```bash
cargo mobench ci run \
  --target android \
  --function sample_fns::fibonacci \
  --devices "Google Pixel 7-13.0" \
  --release \
  --fetch \
  --fetch-output-dir target/browserstack
```

## Fetch Later

```bash
cargo mobench fetch \
  --target android \
  --build-id <browserstack-build-id> \
  --output-dir target/browserstack \
  --wait \
  --poll-interval-secs 10 \
  --timeout-secs 1800
```

Targets:

- `android`: Espresso/App Automate artifacts.
- `ios`: XCUITest/App Automate artifacts.

## Output Layout

Fetched artifacts are written under the selected output directory, defaulting to
`target/browserstack`.

Common files include:

- Build/session JSON.
- Device logs.
- Instrumentation logs when BrowserStack exposes them.
- Video URLs or downloaded video artifacts when available.
- Other BrowserStack session URLs mobench can safely download.

Authenticated downloads are restricted to BrowserStack HTTPS hosts.

## Summarize Existing Results

```bash
cargo mobench summary target/mobench/results.json
cargo mobench summary --format json target/mobench/results.json
cargo mobench summary --format csv target/mobench/results.json
```

Render Markdown from CI summary JSON:

```bash
cargo mobench report summarize \
  --summary target/mobench/ci/summary.json \
  --plots auto
```

## Troubleshooting

- Build still running: use `--wait`.
- Timeout: increase `--timeout-secs`.
- Missing logs: check the BrowserStack session status dashboard.
- No benchmark JSON: inspect device logs for generated runner output markers.
