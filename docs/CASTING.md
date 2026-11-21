# Casting

Limusic can play on a Chromecast, a Google Home or Nest speaker, a speaker group, or a TV with
Google Cast built in. Open the device picker in the player bar (next to the queue button) and pick
the device: the music moves there from where it is, and the bar, the media keys, the global
hotkeys and the OS media controls now control the device. Pick "This computer" to bring it back.

## How it works

Limusic talks to the device directly over the local network, with no Google account and no app to
install on it. The device runs Google's built-in Default Media Receiver, which shows the title, the
artist and the cover. The device keeps time and plays the songs; Limusic keeps the queue and hands
the device the next song before the current one ends, so they follow each other without a gap.

The audio itself comes from this computer. YouTube's stream addresses only work for the address
that asked for them, so the device fetches each song from Limusic, which fetches it from YouTube the
same way it does for local playback. That is also why this computer has to stay awake and on the
network while casting. Limusic keeps it from going to sleep while a cast is playing.

Playing, pausing, skipping and changing the volume on the device itself (the Google Home app, a
voice command, the device's buttons) all work, and Limusic follows along.

## Troubleshooting

- **No devices in the picker.** Your computer and the device have to be on the same network.
  Guest networks and routers with "AP isolation" or "client isolation" turned on keep devices
  from seeing each other. A VPN that routes all traffic can do the same.
- **Windows asks about network access.** The first time the picker looks for devices, and the
  first time you cast, Windows Defender Firewall asks whether Limusic may use the network. Allow
  it. Windows marks new Wi-Fi networks as public, so if your home network is set to public, tick
  that box too, or change the network to private in Settings > Network & internet. If you pressed
  Cancel, open Windows Defender Firewall > Allow an app through firewall and allow Limusic there.
- **macOS asks about the local network.** Allow it. You can change your answer later in System
  Settings > Privacy & Security > Local Network.
- **"couldn't get the music from Limusic".** The device can reach Limusic's controls but not the
  audio: almost always a firewall on this computer, or a VPN between the two.
- **Linux.** Fedora's default firewall allows it. If you turned on a firewall such as `ufw`,
  allow incoming connections from your local network while casting.

## Limitations

- Local files, live broadcasts and karaoke play on this computer only. While casting, local files
  and live broadcasts are skipped, and karaoke is switched off.
- Music videos are not cast; the device plays the audio.
- Tempo, pitch, reverb, the equalizer, crossfade and volume normalization are this computer's and
  are hidden or skipped while casting.
- Casting and Listen Together can't run at the same time.
- With the Default Media Receiver, the Google Home app and Nest Hub screens show play, pause and
  seek but no next or previous buttons. The device's notification on Android and voice commands
  still skip.
