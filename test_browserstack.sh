#!/bin/bash
set -e

# Load credentials
export $(cat .env.local | xargs)

echo "=== BrowserStack Manual Test Run ==="
echo ""

# Upload Android APK
echo "1. Uploading Android APK..."
ANDROID_APP_URL=$(curl -s -u "$BROWSERSTACK_USERNAME:$BROWSERSTACK_ACCESS_KEY" \
  -X POST "https://api-cloud.browserstack.com/app-automate/upload" \
  -F "file=@android/app/build/outputs/apk/debug/app-debug.apk" \
  | jq -r '.app_url')
echo "Android app uploaded: $ANDROID_APP_URL"
echo ""

# Upload Android test APK
echo "2. Uploading Android test APK..."
ANDROID_TEST_URL=$(curl -s -u "$BROWSERSTACK_USERNAME:$BROWSERSTACK_ACCESS_KEY" \
  -X POST "https://api-cloud.browserstack.com/app-automate/espresso/v2/test-suite" \
  -F "file=@android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk" \
  | jq -r '.test_suite_url')
echo "Android test suite uploaded: $ANDROID_TEST_URL"
echo ""

# Trigger Android Espresso run
echo "3. Triggering Android test on Pixel 7..."
ANDROID_BUILD=$(curl -s -u "$BROWSERSTACK_USERNAME:$BROWSERSTACK_ACCESS_KEY" \
  -X POST "https://api-cloud.browserstack.com/app-automate/espresso/v2/build" \
  -d "{\"app\": \"$ANDROID_APP_URL\", \"testSuite\": \"$ANDROID_TEST_URL\", \"devices\": [\"Google Pixel 7-13.0\"], \"project\": \"mobench-test\", \"deviceLogs\": true}" \
  -H "Content-Type: application/json")
echo "Android build started:"
echo "$ANDROID_BUILD" | jq '.'
ANDROID_BUILD_ID=$(echo "$ANDROID_BUILD" | jq -r '.build_id')
echo ""

# Upload iOS app
echo "4. Uploading iOS app..."
IOS_APP_URL=$(curl -s -u "$BROWSERSTACK_USERNAME:$BROWSERSTACK_ACCESS_KEY" \
  -X POST "https://api-cloud.browserstack.com/app-automate/xcuitest/v2/app" \
  -F "file=@target/ios/BenchRunner.zip" \
  | jq -r '.app_url')
echo "iOS app uploaded: $IOS_APP_URL"
echo ""

# Upload iOS test suite
echo "5. Uploading iOS test suite..."
IOS_TEST_URL=$(curl -s -u "$BROWSERSTACK_USERNAME:$BROWSERSTACK_ACCESS_KEY" \
  -X POST "https://api-cloud.browserstack.com/app-automate/xcuitest/v2/test-suite" \
  -F "file=@target/ios/BenchRunnerUITests.zip" \
  | jq -r '.test_suite_url')
echo "iOS test suite uploaded: $IOS_TEST_URL"
echo ""

# Trigger iOS XCUITest run
echo "6. Triggering iOS test on iPhone 14..."
IOS_BUILD=$(curl -s -u "$BROWSERSTACK_USERNAME:$BROWSERSTACK_ACCESS_KEY" \
  -X POST "https://api-cloud.browserstack.com/app-automate/xcuitest/v2/build" \
  -d "{\"app\": \"$IOS_APP_URL\", \"testSuite\": \"$IOS_TEST_URL\", \"devices\": [\"iPhone 14-16\"], \"project\": \"mobench-test\", \"deviceLogs\": true}" \
  -H "Content-Type: application/json")
echo "iOS build started:"
echo "$IOS_BUILD" | jq '.'
IOS_BUILD_ID=$(echo "$IOS_BUILD" | jq -r '.build_id')
echo ""

echo "=== Test runs triggered ==="
echo "Android build ID: $ANDROID_BUILD_ID"
echo "iOS build ID: $IOS_BUILD_ID"
echo ""
echo "Monitor at:"
echo "  Android: https://app-automate.browserstack.com/dashboard/v2/builds/$ANDROID_BUILD_ID"
echo "  iOS: https://app-automate.browserstack.com/dashboard/v2/builds/$IOS_BUILD_ID"
