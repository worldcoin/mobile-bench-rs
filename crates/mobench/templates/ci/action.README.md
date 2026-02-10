# mobench GitHub Action

Run `mobench run` in GitHub Actions with caching, Android SDK setup, and artifact upload.

## Usage

```yaml
- uses: ./.github/actions/mobench
  with:
    run-args: >
      --target android
      --function sample_fns::fibonacci
      --iterations 30
      --warmup 5
      --devices "Google Pixel 7-13.0"
      --release
      --fetch
      --summary-csv
    ci: true
    ndk-version: "26.1.10909125"
  env:
    BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
    BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
```

## Inputs

- `command`: command to invoke (default: `cargo mobench run`).
- `run-args`: arguments passed to `mobench run`.
- `ci`: append `--ci` to enable job summaries + regression exit codes.
- `install-mobench`: install `mobench` with cargo-binstall/cargo install.
- `mobench-version`: optional version to install.
- `install-cargo-ndk`: install `cargo-ndk` for Android builds.
- `setup-android`: install Android SDK/NDK packages.
- `ndk-version`: Android NDK version (used for setup + `ANDROID_NDK_HOME`).
- `android-sdk-root`: Android SDK root directory on the runner.
- `android-packages`: SDK packages list for `setup-android`.
- `cache-cargo`: cache cargo registry/git and `target`.
- `cache-target`: cache `target/` (can be large).
- `cache-gradle`: cache `~/.gradle` wrapper and caches.
- `cache-android`: cache Android SDK/NDK.
- `artifact-name`: artifact name.
- `artifact-path`: paths to upload.
