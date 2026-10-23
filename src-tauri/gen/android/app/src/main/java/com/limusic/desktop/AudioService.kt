package com.limusic.desktop

import android.content.Intent
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.PlaybackParameters
import androidx.media3.common.Player
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import org.json.JSONObject

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

        // ForwardingPlayer enables next/previous track commands from MediaSession / headset / notification
        val forwardingPlayer = object : ForwardingPlayer(exo) {
            override fun seekToNext() {
                AudioBridge.onNext()
            }

            override fun seekToPrevious() {
                AudioBridge.onPrevious()
            }

            override fun seekToNextMediaItem() {
                AudioBridge.onNext()
            }

            override fun seekToPreviousMediaItem() {
                AudioBridge.onPrevious()
            }

            override fun isCommandAvailable(command: Int): Boolean {
                if (command == Player.COMMAND_SEEK_TO_NEXT ||
                    command == Player.COMMAND_SEEK_TO_PREVIOUS ||
                    command == Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM ||
                    command == Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM) {
                    return true
                }
                return super.isCommandAvailable(command)
            }

            override fun getAvailableCommands(): Player.Commands {
                return super.getAvailableCommands().buildUpon()
                    .add(Player.COMMAND_SEEK_TO_NEXT)
                    .add(Player.COMMAND_SEEK_TO_PREVIOUS)
                    .add(Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
                    .add(Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
                    .build()
            }
        }

        mediaSession = MediaSession.Builder(this, forwardingPlayer).build()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? {
        return mediaSession
    }

    fun loadUrl(url: String, startPositionMs: Long, headersJson: String? = null) {
        player?.let { p ->
            val httpFactory = DefaultHttpDataSource.Factory()
            if (!headersJson.isNullOrEmpty()) {
                try {
                    val json = JSONObject(headersJson)
                    val map = mutableMapOf<String, String>()
                    val keys = json.keys()
                    while (keys.hasNext()) {
                        val key = keys.next()
                        map[key] = json.getString(key)
                    }
                    httpFactory.setDefaultRequestProperties(map)
                } catch (e: Exception) {
                    android.util.Log.e("AudioService", "Error parsing headers JSON", e)
                }
            }
            val mediaSourceFactory = DefaultMediaSourceFactory(this).setDataSourceFactory(httpFactory)
            val mediaItem = MediaItem.fromUri(url)
            val mediaSource = mediaSourceFactory.createMediaSource(mediaItem)
            p.setMediaSource(mediaSource, startPositionMs)
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

    fun setPlaybackSpeed(speed: Float) {
        player?.playbackParameters = PlaybackParameters(speed)
    }

    fun setLoop(loop: Boolean) {
        player?.repeatMode = if (loop) Player.REPEAT_MODE_ONE else Player.REPEAT_MODE_OFF
    }

    fun updateMetadata(title: String, artist: String, album: String?, coverUrl: String?) {
        val metaBuilder = MediaMetadata.Builder()
            .setTitle(title)
            .setArtist(artist)
        if (album != null) {
            metaBuilder.setAlbumTitle(album)
        }
        if (coverUrl != null) {
            metaBuilder.setArtworkUri(Uri.parse(coverUrl))
        }
        val metadata = metaBuilder.build()
        player?.let { p ->
            p.currentMediaItem?.let { current ->
                val updated = current.buildUpon().setMediaMetadata(metadata).build()
                p.replaceMediaItem(p.currentMediaItemIndex, updated)
            }
        }
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
