# Keep JNI bridge methods
-keepclasseswithmembernames class * {
    native <methods>;
}

-keep class com.limusic.desktop.AudioBridge { *; }
-keep class com.limusic.desktop.AudioService { *; }
-keep class app.tauri.** { *; }
