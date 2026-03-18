# mobench GitHub Action

Run `mobench ci run` in GitHub Actions with caching, Android SDK setup, and artifact upload.

## Usage

```yaml
- uses: ./.github/actions/mobench
  with:
    command: cargo mobench ci run
    run-args: |
      --target android
      --function sample_fns::fibonacci
      --iterations 30
      --warmup 5
      --devices "Google Pixel 7-13.0"
      --release
      --fetch
    ci: false
    ndk-version: "26.1.10909125"
  env:
    BROWSERSTACK_USERNAME: ${{ secrets.BROWSERSTACK_USERNAME }}
    BROWSERSTACK_ACCESS_KEY: ${{ secrets.BROWSERSTACK_ACCESS_KEY }}
```

## Inputs

- `command`: command to invoke. Supported values are `cargo mobench ci run` (default) and `cargo mobench run`.
- `run-args`: arguments passed to `command`. Use quoted values for arguments containing spaces (for example device names).
- `ci`: append `--ci` only when `command` is exactly `cargo mobench run`; ignored for `cargo mobench ci run`.
- `install-mobench`: install `mobench` with cargo-binstall/cargo install.
- `mobench-version`: optional version to install. Set `0.1.16` to pin this release explicitly.
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
- `pr-comment`: publish sticky PR comment from CI summary (`true|false`).
- `pr-number`: PR number override (optional).
- `pr-comment-marker`: sticky comment marker used for idempotent updates.
- `github-token`: token for PR comment publishing.

## Notes

- Inputs are passed through environment variables in shell steps to reduce script-injection risk from workflow inputs.
- `command` is allow-listed in the action implementation; unsupported command values fail the job early.

## Cache keys

The action uses deterministic cache keys:
- Cargo cache: `${runner.os}-cargo-${hashFiles('**/Cargo.lock')}`
- Target cache: `${runner.os}-target-${hashFiles('**/Cargo.lock')}`
- Gradle cache: `${runner.os}-gradle-${hashFiles('**/*.gradle*', '**/gradle/wrapper/gradle-wrapper.properties', '**/gradle.properties')}`
- Android SDK cache: `${runner.os}-android-${inputs.ndk-version}`

## PR comment mode

To enable sticky PR comments, grant workflow permissions and pass token:

```yaml
permissions:
  contents: read
  pull-requests: write

- uses: ./.github/actions/mobench
  with:
    pr-comment: true
    github-token: ${{ github.token }}
```
