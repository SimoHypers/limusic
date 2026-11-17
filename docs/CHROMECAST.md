# Chromecast casting: troubleshooting and limits

Limusic can cast audio directly to Google Cast and Chromecast devices on your local network.

## Troubleshooting device discovery and connection

If your Chromecast device does not appear in the cast menu or fails to connect:

- **Same network / subnet:** Both your computer running Limusic and the Chromecast device must be connected to the exact same Wi-Fi network and IP subnet.
- **AP / Client Isolation:** Ensure that Access Point (AP) isolation, client isolation, or guest mode is disabled on your Wi-Fi router. Isolation prevents wireless devices on the same network from communicating with each other.
- **VPNs:** Active VPN connections often block mDNS discovery packets and local network routing. Turn off your VPN when attempting to cast.
- **Firewall:** Ensure Limusic is permitted through your operating system's firewall (such as Windows Defender Firewall) on private networks. mDNS uses UDP port 5353, and Cast communication uses local TCP ports.
- Windows asks about network access the first time you cast: allow Limusic on private networks (if you pressed Cancel, remove the block rules for Limusic in Windows Defender Firewall)

## Limitations

The Chromecast implementation has the following limitations:
- **No local files:** Only tracks streamed from YouTube Music / YouTube are supported. Local audio files cannot be cast.
- **No karaoke:** Active vocal removal (karaoke processing) runs locally on your computer and cannot be streamed to a Cast receiver.
- **No live streams / live radio:** Live broadcasts and active live streams are not supported for casting.
