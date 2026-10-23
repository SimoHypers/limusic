package com.limusic.desktop

import android.content.Context
import android.content.Intent
import android.os.Handler
import android.os.Looper

/**
 * Singleton bridge connecting Rust (crates/player/src/android.rs) with Android's AudioService.
 */
object AudioBridge {
    private val handler = Handler(Looper.getMainLooper())
    private var service: AudioService? = null
    private var context: Context? = null

    init {
        try {
            System.loadLibrary("app_lib")
        } catch (e: UnsatisfiedLinkError) {
            android.util.Log.w("AudioBridge", "Could not load app_lib JNI: ${e.message}")
        }
    }

    fun attachContext(ctx: Context) {
        context = ctx.applicationContext
    }

    fun attachService(svc: AudioService) {
        service = svc
    }

    fun detachService() {
        service = null
    }

    // --- Calls from Rust into Android ---

    @JvmStatic
    fun play() {
        handler.post { service?.play() }
    }

    @JvmStatic
    fun pause() {
        handler.post { service?.pause() }
    }

    @JvmStatic
    fun stop() {
        handler.post { service?.stop() }
    }

    @JvmStatic
    fun seek(positionSecs: Double) {
        handler.post { service?.seekTo((positionSecs * 1000).toLong()) }
    }

    @JvmStatic
    fun load(url: String, startSecs: Double, headersJson: String? = null) {
        handler.post {
            ensureServiceStarted()
            service?.loadUrl(url, (startSecs * 1000).toLong(), headersJson)
        }
    }

    @JvmStatic
    fun setSpeed(speed: Float) {
        handler.post { service?.setPlaybackSpeed(speed) }
    }

    @JvmStatic
    fun setLoop(loop: Boolean) {
        handler.post { service?.setLoop(loop) }
    }

    @JvmStatic
    fun setMetadata(title: String, artist: String, album: String?, coverUrl: String?) {
        handler.post { service?.updateMetadata(title, artist, album, coverUrl) }
    }

    @JvmStatic
    fun setVolume(volumePercent: Int) {
        handler.post {
            val vol = (volumePercent.coerceIn(0, 100) / 100f)
            service?.setVolume(vol)
        }
    }

    private fun ensureServiceStarted() {
        val ctx = context ?: return
        val intent = Intent(ctx, AudioService::class.java)
        ctx.startService(intent)
    }

    // --- Calls from Android into Rust JNI ---

    external fun nativeOnPosition(position: Double)
    external fun nativeOnDuration(duration: Double)
    external fun nativeOnPlaying(playing: Boolean)
    external fun nativeOnPaused(paused: Boolean)
    external fun nativeOnTrackEnded()
    external fun nativeOnTrackFailed()
    external fun nativeOnNext()
    external fun nativeOnPrevious()
    external fun nativeOnToggle()

    fun onPosition(positionSecs: Double) {
        try {
            nativeOnPosition(positionSecs)
        } catch (e: UnsatisfiedLinkError) {
            // Handled during pre-initialization or mock runs
        }
    }

    fun onDuration(durationSecs: Double) {
        try {
            nativeOnDuration(durationSecs)
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onPlaying(playing: Boolean) {
        try {
            nativeOnPlaying(playing)
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onPaused(paused: Boolean) {
        try {
            nativeOnPaused(paused)
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onTrackEnded() {
        try {
            nativeOnTrackEnded()
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onTrackFailed() {
        try {
            nativeOnTrackFailed()
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onNext() {
        try {
            nativeOnNext()
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onPrevious() {
        try {
            nativeOnPrevious()
        } catch (e: UnsatisfiedLinkError) {
        }
    }

    fun onToggle() {
        try {
            nativeOnToggle()
        } catch (e: UnsatisfiedLinkError) {
        }
    }
}
