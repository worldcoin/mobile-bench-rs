# Android demo app

Minimal Android app that loads the Rust `sample-fns` cdylib and calls exported functions. This is a thin wrapper meant for BrowserStack AppAutomate and CI smoke tests.

## Build steps

1. Build Rust libs for Android (outputs to `target/mobench/android/`):
   ```bash
   cargo mobench build --target android
   ```

2. Assemble the APK (requires Java + Gradle + Android SDK/NDK on PATH):
   ```bash
   cd target/mobench/android
   ./gradlew :app:assembleDebug
   ```

Artifacts will be under `target/mobench/android/app/build/outputs/apk/debug/`.

## Additional CLI options

```bash
# Preview build without making changes
cargo mobench build --target android --dry-run

# Build with verbose output
cargo mobench build --target android --verbose

# Build to custom output directory
cargo mobench build --target android --output-dir ./my-output
```

> Note: Gradle/AGP versions are pinned in the generated `build.gradle`. Update as needed.
