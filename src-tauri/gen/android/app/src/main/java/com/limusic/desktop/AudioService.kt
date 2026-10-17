package com.limusic.desktop

import android.content.Intent
import android.os.Handler
import android.os.Looper
import androidx.media3.common.MediaItem
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

class AudioService : MediaSessionService() {
    private var mediaSession: MediaSession? = null
    private var player: ExoPlayer? = null
    private val handler = Handler(Looper.getMainLooper())
    private val progressRunnable = object : Runnable {
        override fun run() {
            player?.let { p ->
                if (p.isPlaying) {
                    val posSecs = p.currentPosition / 1000.0
                    AudioBridge.onPosition(posSecs)
                }
            }
            handler.postDelayed(this, 250)
        }
    }

    override fun onCreate() {
        super.onCreate()
        AudioBridge.attachService(this)

        val exo = ExoPlayer.Builder(this).build()
        exo.addListener(object : Player.Listener {
            override fun onPlaybackStateChanged(playbackState: Int) {
                when (playbackState) {
                    Player.STATE_READY -> {
                        val durationSecs = exo.duration / 1000.0
                        if (durationSecs > 0) {
                            AudioBridge.onDuration(durationSecs)
                        }
                    }
                    Player.STATE_ENDED -> {
                        AudioBridge.onTrackEnded()
                    }
                    Player.STATE_IDLE -> {
                        // Stopped or error
                    }
                }
            }

            override fun onIsPlayingChanged(isPlaying: Boolean) {
                AudioBridge.onPlaying(isPlaying)
                AudioBridge.onPaused(!isPlaying)
                if (isPlaying) {
                    handler.post(progressRunnable)
                } else {
                    handler.removeCallbacks(progressRunnable)
                }
            }

            override fun onPlayerError(error: androidx.media3.common.PlaybackException) {
                AudioBridge.onTrackFailed()
            }
        })

        player = exo
        mediaSession = MediaSession.Builder(this, exo).build()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? {
        return mediaSession
    }

    fun loadUrl(url: String, startPositionMs: Long) {
        player?.let { p ->
            val mediaItem = MediaItem.fromUri(url)
            p.setMediaItem(mediaItem, startPositionMs)
            p.prepare()
            p.play()
        }
    }

    fun play() {
        player?.play()
    }

    fun pause() {
        player?.pause()
    }

    fun stop() {
        player?.stop()
        handler.removeCallbacks(progressRunnable)
    }

    fun seekTo(positionMs: Long) {
        player?.seekTo(positionMs)
    }

    fun setVolume(vol: Float) {
        player?.volume = vol
    }

    override fun onDestroy() {
        handler.removeCallbacks(progressRunnable)
        AudioBridge.detachService()
        mediaSession?.run {
            player.release()
            release()
            mediaSession = null
        }
        super.onDestroy()
    }
}
