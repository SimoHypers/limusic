# Keep JNI bridge methods
-keepclasseswithmembernames class * {
    native <methods>;
}

-keep class com.limusic.desktop.** { *; }
-keep class app.tauri.** { *; }
