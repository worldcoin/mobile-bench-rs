# ProGuard rules for mobench Android benchmark app
# These rules ensure UniFFI and JNA work correctly when minification is enabled.

# Keep JNA classes for UniFFI
-keep class com.sun.jna.** { *; }
-keep class * implements com.sun.jna.** { *; }

# Keep UniFFI generated bindings
-keep class uniffi.** { *; }

# Keep application benchmark classes
-keepclassmembers class * {
    @uniffi.* <methods>;
}

# Keep native method names (required for JNI)
-keepclasseswithmembernames class * {
    native <methods>;
}

# Keep Kotlin metadata for reflection
-keep class kotlin.Metadata { *; }
