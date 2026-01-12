# Keep all native method names so JNI bindings stay intact
-keepclasseswithmembernames class * {
    native <methods>;
}
