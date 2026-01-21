# Android demo app

Minimal Android app that loads the Rust `sample-fns` cdylib and calls exported functions. This is a thin wrapper meant for BrowserStack AppAutomate and CI smoke tests.

## Build steps
1. Build Rust libs for Android:
   ```bash
   cargo mobench build --target android
   ```
2. Assemble the APK (requires Java + Gradle + Android SDK/NDK on PATH):
   ```bash
   cd android
   gradle :app:assembleDebug
   ```

Artifacts will be under `android/app/build/outputs/apk/debug/`.

> Note: Gradle/AGP versions are pinned in `android/build.gradle`. Update as needed.
