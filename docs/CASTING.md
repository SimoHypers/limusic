# Casting

Limusic can play on a device on your network: a Chromecast, a Google Home or Nest speaker, or
any DLNA renderer. The button is in the player bar, next to the queue button: it opens a picker
with the devices that were found, and the one you pick takes the music over from this desktop.

While a device is playing, the bar shows its name, the transport controls the device, and the
queue moves on there: when the track ends, Limusic hands the next one over.

A Cast device keeps a queue of its own. Limusic hands it a few already-played tracks, the track
that is playing, and the next few resolved ones, and that is what lets the device's next,
previous, shuffle and repeat controls work where the receiver offers them. When the receiver
moves inside that window, Limusic follows: the queue pointer, the now-playing line and the
listening history move with it. When playback nears either end of the window, the app rebuilds
it around the playing track and hands it over at the same position, so next and previous keep
working instead of running out. A rebuild can leave a short buffering pause on the device.
Which surfaces draw those controls depends on the receiver application; Google's built-in
receiver has limits, covered in
[The receiver application](#the-receiver-application-branding-and-controls).

A DLNA renderer has no queue. It plays the one track it is given, and Limusic decides what comes
next there. For a renderer the app treats a track as over from the length it resolved, not from
the device saying so: a renderer says "stopped" or "idle" while it buffers, and reading that as
the end would cut the track short.

Before a new track is handed over, the device is told to stop what it was playing and given a
moment to let go of it — a load that arrives while it is still tearing the old one down is
refused, and then it refuses the next one as well. A burst of skips is handled the same way: the
loads wait for the device to be ready, and a load that is still waiting is replaced by a newer
one, because the queue is the truth and the device only has to end up on the track the queue is
on.

## What is supported

| Device | Found by | Controlled over |
|---|---|---|
| Chromecast, Google Home, Nest speakers | mDNS (`_googlecast._tcp.local`) | CastV2 over TLS :8009 |
| DLNA / UPnP MediaRenderer | SSDP | SOAP `AVTransport` |

The Cast devices are found by their mDNS announcement, which every Chromecast and every Google
speaker makes. The DLNA renderers answer an SSDP search, which is how a television or a sound
bar announces that it can play a file.

Both have to be on the same network as this computer, and this computer has to be able to reach
the device. A guest Wi-Fi or a VPN that routes everything elsewhere will not do.

The DLNA path is written against the UPnP `AVTransport` and `RenderingControl` services but has
not been validated on a physical renderer yet.

## How the music gets to the device

The two kinds of device want the song from different places, and the difference is not a
preference: it is what each one will accept.

**A Cast device fetches it from YouTube.** The receiver refuses a URL on a private address
before it even opens a connection — it answers `LOAD_FAILED` without a packet crossing the
network — so the address it is given has to be a public one. That is YouTube's own stream
address, and the device fetches it from Google's servers. The one thing it cannot carry is the
User-Agent the stream was signed under; YouTube's servers do not insist on it, and the song
plays.

**A DLNA renderer fetches it from this computer.** A renderer speaks `http://` and not
`https://`, and it expects a URL on the network, so the bytes come through the loopback audio
proxy published on the LAN for the occasion (`audioproxy.rs`). The proxy answers with the
headers the stream was signed under, and asks YouTube for bounded ranges, which are served at
full speed where an open-ended request is throttled to about twice the length of the song.

The LAN listener is bound when the first renderer needs it, and the address it hands out
carries a random token that is generated per launch. Only the device that was picked is ever
given an address with that token in it.

The track is resolved as AAC in an MP4 container for the device, not as the Opus in a WebM
container the local player uses: cast devices and renderers play the first and refuse the
second. A renderer that only speaks MP3 is not supported — there is no transcoding.

The few upcoming tracks in a Cast receiver's queue are resolved ahead of time for the same
reason: the receiver cannot start what it has not fetched, and resolving when the track ends
would leave a gap between songs. The history behind the playing track is resolved for the
device too, so Previous reaches tracks that were played here before the cast started; the cast
cache makes a rebuild cheap after the first one. The window is small, so the resolves are
bounded and the URLs are still fresh when they are used.

## What the app does while a device is playing

- The local player is paused. The device is the one making the sound.
- Play, pause, seek and the volume go to the device, from the bar, from the media keys and from
  the operating system's media widget. The volume is the speaker's own, and the bar follows the
  level the device reports back.
- Next, previous, shuffle and repeat work through the receiver's own queue, but which surfaces
  offer them depends on the receiver application. With Google's built-in receiver the Android
  media notification offers them, and a voice "next"/"previous" targets the same queue. The
  Google Home app and a Nest Hub touch screen do not draw those buttons for it, because the
  built-in receiver does not advertise queue commands; see
  [The receiver application](#the-receiver-application-branding-and-controls).
  Repeat-all wraps in the app's queue rather than in the receiver's window, and repeat-one loops
  on the receiver. A DLNA renderer has no queue, so it offers none of them.
- Everything that moves the music goes through the same place, so the media keys, the global
  hotkeys, the media widget and a linked phone all reach the device while it is playing — and
  this desktop while it is not.
- Queue edits made here while a cast is running reach the device at the next handover or
  window rebuild: the receiver keeps the window of tracks it was given until then.
- The track's position comes from the device. It reports every few seconds, so the app carries
  the position forward in between and the clock runs instead of stepping. When the device stops,
  the clock stops where it had got to — the time since its last word belongs to the track. The device's own
  length is shown too, so the seek bar is the song's, not the one this desktop last played.
- This desktop's player keeps running in the background while a device plays — paused, but mpv
  still reports its position — so what it says does not reach the windows.
- The song is registered in your listening history and scrobbled as usual.
- Tempo, pitch and reverb are this desktop's own, and are hidden while a device is playing:
  they are applied by the local player, which is not the one playing.
- Quitting the app stops the device — through the tray's Quit, through `kill`, and through the
  window closing for real. A speaker left running on a song nobody is listening to is the wrong
  kind of cast.

Stopping the cast stops the device and brings the music back here, at the position the device
had reached — with this desktop's own volume, and playing only if it was playing before the
device took over. A track that was paused here stays paused, and it is loaded paused rather than
played for a moment first.

## What the phone shows

The Google Home app shows the receiver application and what is playing. Limusic identifies
itself as the sender in the connection's user agent, and the song is sent with every track: its
title, the artist, the album and the cover art, at a size worth looking at.

## The receiver application: branding and controls

Google's built-in receiver has the registered application ID `CC1AD845` and the fixed name
**Default Media Receiver**. Fields in the Cast connection describe the sender; they cannot
rename the receiver application. A sender that uses `CC1AD845` will therefore show that name
on the Cast device and in the Google Home app.

### What the built-in receiver does not offer

Google's receiver advertises pause, seek, volume, mute, edit-tracks and playback-rate as its
supported commands, but not the queue commands. It still accepts queue jumps, which is why the
notification's next and previous work, but the user interfaces that draw controls from that list
hide them:

- **The Google Home app's device page and a Nest Hub's touch controls show no next, previous,
  shuffle or repeat buttons.** Play, pause and seek stay available there.
- **The Android media notification offers next and previous.** They move the receiver's queue,
  and the app follows to the same track. A voice "next" or "previous" addresses that same
  queue.
- **The device shows "Default Media Receiver"**, not Limusic, while it is launching or idle.
  The track's title, artist, album and cover are still shown while it plays.
- **Repeat-all is the app's.** The receiver only holds a window of the queue, so the app wraps at
  the end of it rather than letting the receiver loop that window.

None of this is configurable from the sender: the receiver application decides what it
advertises, and Google's application is fixed.

### What a registered receiver changes

A receiver application owned by the project fixes the name and the control list together:

1. The upstream project owner registers a Google Cast Developer Console account. The developer
   account registration fee is **$5 USD, non-refundable**.
2. They register a **Custom Web Receiver** named **Limusic** at an HTTPS URL the project controls.
   A custom receiver is what can advertise `QUEUE_NEXT`, `QUEUE_PREV`, `QUEUE_SHUFFLE`,
   `QUEUE_REPEAT_ALL` and `QUEUE_REPEAT_ONE` in `CastReceiverOptions.supportedCommands`, which
   is what makes the Google Home app and a Nest Hub draw the controls. A Styled Media Receiver
   can change the name and the styling, but it does not expose that control list.
3. The receiver is hosted with the project's website, and its URL is entered in the developer
   console.
4. They copy the generated public Application ID into `LIMUSIC_CAST_APP_ID` for local testing,
   and into `src-tauri/src/cast/chromecast.rs` for bundled releases.

The registration belongs to the upstream maintainer or organization so that ownership,
continuity, and the Application ID do not depend on an individual contributor's Google account.
A fork that wants its own branding must register and host its own receiver instead. Until that
registration exists, Limusic uses the built-in receiver and the limitations above apply.

Google's registration, Styled Media Receiver and Custom Web Receiver documentation is at
<https://developers.google.com/cast/docs/registration>,
<https://developers.google.com/cast/docs/styled_receiver> and
<https://developers.google.com/cast/docs/web_receiver/basic>. The developer console is at
<https://cast.google.com/publish>.

## Things that do not travel

- **Karaoke** is done on this computer, so it does not apply to a cast.
- **Music videos** are drawn by the local player, so a cast plays the audio only.
- **A local file** has no stream address to hand over, so it cannot be cast.
- **A live broadcast** is an HLS playlist rather than a file, so it cannot be cast.
- **Your own uploads** stream only with the account's cookie, and a device sends none.
- **The effects** (tempo, pitch, reverb) are applied locally, as above.
- **The device's own queue** is not adopted: picking a device hands it Limusic's queue, not
  whatever the speaker was playing before.
