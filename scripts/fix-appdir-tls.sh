#!/usr/bin/env bash
# Repair the bundled AppDir so it runs on hosts that aren't the build machine, then repack and
# re-sign the AppImage in place.
#
# Six defects. The first five are linuxdeploy's; the sixth is Ubuntu's gnutls.
#
#   1. GIO_EXTRA_MODULES is written with a literal newline in it, plus an absolute path into the
#      build machine's own target/ directory. GLib parses one garbage path, finds no module, and
#      the webview falls back to GDummyTlsBackend (supports_tls=False): no HTTPS at all, so no
#      thumbnails, no player.js, no PoToken, and mpv gets handed a dead URL. Fixed by pointing it
#      at the AppDir's own module directories.
#
#   2. libjack.so.0 is on linuxdeploy's excludelist (it normally must match the host's JACK), but
#      libmpv.so.2 and libavdevice.so.60 DT_NEED it — so on a host with no JACK at all the app
#      cannot even start: "error while loading shared libraries: libjack.so.0". Fixed by bundling
#      it. That also makes the host's pipewire-jack shim unreachable, which is what killed v0.2.10:
#      the shim came from the host, wanted pw_log_topic_register from pipewire 1.6, and resolved
#      against the pipewire 1.0 in the bundle.
#
#   3. libwayland-client.so.0 gets bundled even though it is on the upstream AppImage excludelist,
#      and that one library breaks the app on every Mesa host (KI-9). Ubuntu 24.04 ships wayland
#      1.22; Mesa 25/26 as shipped by Debian testing, Arch and Fedora 44 needs wl_fixes_interface,
#      wl_display_create_queue_with_name and wl_display_dispatch_queue_timeout, all wayland >= 1.23.
#      The AppDir is first on LD_LIBRARY_PATH, so when the WebKit web process dlopens the host's
#      libEGL_mesa.so.0 it resolves against our 1.22 and fails on those three symbols. libglvnd then
#      has no vendor, eglGetDisplay returns EGL_NO_DISPLAY, and WebKit kills the process:
#      "Could not create default EGL display: EGL_BAD_PARAMETER. Aborting..." — once per webview,
#      so the window and both hidden webviews are dead and the app does nothing at all. Invisible on
#      NVIDIA, whose EGL vendor library does not use those symbols, which is why it survived every
#      test on the maintainer's desktop.
#
#   4. GLib scans its compiled-in module directory in addition to GIO_EXTRA_MODULES, and for an
#      Ubuntu build that is /usr/lib/x86_64-linux-gnu/gio/modules — a path that also exists on the
#      user's Debian or Ubuntu host, holding modules built against their much newer GLib. It loads
#      them into our 2.80 and they fail. Fixed by also setting GIO_MODULE_DIR, which replaces the
#      compiled-in path rather than adding to it.
#
#   5. No GStreamer plugin is bundled at all. libwebkit2gtk DT_NEEDs ten GStreamer *core* libraries,
#      so linuxdeploy bundles those and stops there. WebKit plays every <video> through GStreamer,
#      and a registry with no plugins in it has no demuxer, no decoder, not even typefind. The
#      bundled Ubuntu libgstreamer has /usr/lib/x86_64-linux-gnu/gstreamer-1.0 compiled in as its
#      plugin path, which does not exist on Fedora (/usr/lib64/gstreamer-1.0) or Arch
#      (/usr/lib/gstreamer-1.0), so the registry comes up empty, WebKit cannot build a pipeline and
#      calls its own CRASH(). That is what killed the web process on every music video in v0.5.0.
#      Measured against the shipped v0.5.0 AppDir: 1 plugin, against 237 from the host stack that
#      `cargo tauri dev` uses, which is exactly why it only ever failed in the AppImage. Fixed by
#      bundling the plugins that pipeline needs, plus gst-plugin-scanner, and pointing
#      GST_PLUGIN_SYSTEM_PATH_1_0 at them. The host's plugin directory stays out of it for the same
#      reason the host's gio modules do (defect 4): those are built against the host's GStreamer,
#      not the one we ship.
#
#   6. The trust store, which took two goes to get right. Ubuntu's gnutls has
#      /etc/ssl/certs/ca-certificates.crt compiled in as its one and only trust store path, with no
#      env var to redirect it. That path exists on Debian, Ubuntu, Fedora and Arch, which is why
#      bundling gnutls looked settled for eleven releases. openSUSE keeps its bundle at
#      /var/lib/ca-certificates/ca-bundle.pem and has no such file, so our gnutls came up with zero
#      trust anchors and every HTTPS load in the webview failed as "Unacceptable TLS certificate":
#      no sign-in, no thumbnails, no music video (issue #164). Rust is unaffected, it carries
#      rustls, so the app looks alive. Fixed by moving the gnutls stack off the library path so the
#      host's own copy, which knows where the host's trust store is, is what loads. Conditionally:
#      see 1e, Arch cannot serve the chain and keeps ours.
#
# ON PRUNING. v0.2.11 pruned eight libraries and broke worse than what it fixed, so the bar is high,
# but "never prune" is the wrong rule — defect 3 above is only fixable by pruning. What made v0.2.11
# wrong, and what a prune has to clear:
#
#   - Sonames must be portable. v0.2.11 dropped nettle when Arch was on libnettle.so.9 and the
#     bundle's libarchive and libsrt wanted .so.8, which made the app unloadable there. Arch is on
#     .so.9 again today (nettle 4.0), which is why defect 6's prune is conditional at runtime rather
#     than done here with rm. libwayland-client.so.0 is .so.0 on every distro and its ABI only ever
#     gains symbols, so the host's copy always satisfies our 1.22 users.
#   - Direction matters. A prune has to survive the host's copy resolving against *our* remaining
#     libraries, not only ours resolving against theirs. Dropping libgnutls alone fails exactly
#     there: host gnutls 3.8.10+ wants nettle_rsa_oaep_sha384_decrypt@HOGWEED_6, and our nettle 3.9
#     does not have it, so the whole crypto chain under gnutls stands or falls together.
#   - The host must be guaranteed to have it. GTK 3 and Mesa both hard-depend on libwayland-client,
#     so any host that can run a GUI has it. That is also why upstream lists it as a system library.
#   - It must not be a plugin directory. Host gio modules are built against the host's GLib; that
#     half of v0.2.11 is what produced the gvfs errors, and defect 4 is the last of it.
#
# Usage:  scripts/fix-appdir-tls.sh [bundle-dir]     (default: target/release/bundle/appimage)
# Runs from CI (.github/workflows/linux-release.yml) and by hand after a local
# `cargo tauri build --bundles appimage` when you want to test the repaired AppDir.
set -euo pipefail
cd "$(dirname "$0")/.."

BUNDLE="${1:-target/release/bundle/appimage}"
APPDIR="$(readlink -f "$BUNDLE/limusic.AppDir" 2>/dev/null || true)"
[ -n "$APPDIR" ] && [ -d "$APPDIR" ] || {
  echo "no AppDir at $BUNDLE/limusic.AppDir — run \`cargo tauri build --bundles appimage\` first"; exit 1; }
APPIMAGE="$(ls "$BUNDLE"/limusic_*.AppImage 2>/dev/null | head -1 || true)"
[ -n "$APPIMAGE" ] || { echo "no limusic_*.AppImage in $BUNDLE"; exit 1; }
APPIMAGE="$(readlink -f "$APPIMAGE")"

# Libraries that must come from the HOST, never from us. Bundling one of these shadows the host's
# own copy for everything the app dlopens later, which is how v0.2.14 shipped an AppImage that could
# not open a single webview (defect 3). The gnutls stack on the last two lines is the softer case:
# it still travels with us, in gnutls-fallback/, and 1e/2a decide at runtime whose copy loads. It is
# here so bundle_deps_of never puts it back on the default path. Keep in step with HOST_BASELINE in
# .github/workflows/linux-release.yml: that list fails the build when we *fail* to bundle something
# not on it, this one stops us bundling something that is. Same question, opposite sides.
HOST_BASELINE="libGL.so.1 libEGL.so.1 libGLX.so.0 libGLdispatch.so.0 libOpenGL.so.0
  libdrm.so.2 libgbm.so.1 libwayland-client.so.0 libX11.so.6 libX11-xcb.so.1 libxcb.so.1
  libxcb-dri3.so.0 libexpat.so.1 libfontconfig.so.1 libfreetype.so.6 libharfbuzz.so.0
  libfribidi.so.0 libz.so.1 libasound.so.2 libusb-1.0.so.0 libcom_err.so.2 libgpg-error.so.0
  libresolv.so.2 libgcc_s.so.1 libstdc++.so.6
  libgnutls.so.30 libnettle.so.8 libhogweed.so.6 libgmp.so.10 libtasn1.so.6 libp11-kit.so.0
  libidn2.so.0 libunistring.so.5"

# Copy every DT_NEEDED of $1 that the AppDir doesn't already have. glibc and the loader are the
# host's job; everything else has to travel with us, or we just move "cannot open shared object
# file" one library along (jackd2's libjack needs libdb-5.3, which Arch doesn't ship at all).
bundle_deps_of() {
  local of="$1" name path
  while read -r name path; do
    case "$name" in libc.so.*|libm.so.*|libpthread.so.*|libdl.so.*|librt.so.*|ld-linux*) continue;; esac
    case " $(echo "$HOST_BASELINE" | tr -s ' \n' ' ') " in *" $name "*) continue;; esac
    [ -e "$APPDIR/usr/lib/$name" ] && continue
    [ -e "$path" ] || continue
    cp -L "$path" "$APPDIR/usr/lib/$name"
    echo "==> bundled $name (dependency of $(basename "$of"))"
  done < <(ldd "$of" | awk '/=> \//{print $1, $3}')
}

# 1. libjack: bundle the build host's copy. Prefer a real jackd2 libjack over a pipewire-jack shim
#    if the host has both, since the shim drags libpipewire's version coupling back in.
if [ -e "$APPDIR/usr/lib/libjack.so.0" ]; then
  echo "==> libjack already bundled"
else
  JACK=""
  for cand in /usr/lib/x86_64-linux-gnu/libjack.so.0 /usr/lib64/libjack.so.0 /usr/lib/libjack.so.0; do
    [ -e "$cand" ] && { JACK="$cand"; break; }
  done
  # Fallback for hosts that keep it off the default path — Fedora's pipewire-jack lives in
  # /usr/lib64/pipewire-0.3/jack/. CI hits the standard path above and gets jackd2's real libjack;
  # this only matters for local test builds, where the shim pairs with that host's own libpipewire.
  # `|| true`: head closing the pipe early makes ldconfig die of SIGPIPE, which pipefail would
  # otherwise turn into a silent fatal exit 141.
  [ -n "$JACK" ] || JACK="$(ldconfig -p 2>/dev/null | awk '/libjack\.so\.0 /{print $NF}' | head -1 || true)"
  [ -n "$JACK" ] || { echo "libjack.so.0 not found on the build host — install libjack-jackd2-0"; exit 1; }
  cp -L "$JACK" "$APPDIR/usr/lib/libjack.so.0"
  echo "==> bundled libjack.so.0 from $JACK"
  bundle_deps_of "$JACK"
fi

# 1b0. Wrong-architecture gio modules. linuxdeploy copies whatever sits in the host's gio module
#      directory, and on a multilib Fedora /usr/lib/gio/modules holds the *32-bit* build while the
#      real one lives in /usr/lib64. GLib scans modules by basename, so the unloadable 32-bit copy
#      shadows the good one, `g_tls_backend_get_default()` returns GDummyTlsBackend and the webview
#      has no HTTPS at all: the app plays (Rust does its own TLS) but not one thumbnail loads.
#      Defect 1 again, by another route. Checked with the ELF class byte rather than `file`, which
#      is not on every build image.
elf64() { [ "$(od -An -tu1 -j4 -N1 "$1" 2>/dev/null | tr -d ' ')" = 2 ]; }
for m in "$APPDIR"/usr/lib/gio/modules/*.so "$APPDIR"/usr/lib64/gio/modules/*.so; do
  [ -e "$m" ] || continue
  elf64 "$m" && continue
  rm -f "$m"
  echo "==> removed ${m#$APPDIR/} — not x86-64"
done

# 1b. The gio TLS module. linuxdeploy's GTK plugin bundles it on Fedora but not on Ubuntu, so copy
#     it in ourselves rather than depend on that. Only this one module: gvfs and libproxy are built
#     against the host's GLib and blow up against the older one we bundle, which is exactly what
#     v0.2.11 shipped.
if ls "$APPDIR"/usr/lib/gio/modules/libgiognutls.so >/dev/null 2>&1; then
  echo "==> gio TLS module already bundled"
else
  TLSMOD=""
  for dir in "$(pkg-config --variable=giomoduledir gio-2.0 2>/dev/null || true)" \
             /usr/lib/x86_64-linux-gnu/gio/modules /usr/lib64/gio/modules /usr/lib/gio/modules; do
    [ -n "$dir" ] && [ -e "$dir/libgiognutls.so" ] && { TLSMOD="$dir/libgiognutls.so"; break; }
  done
  [ -n "$TLSMOD" ] || { echo "libgiognutls.so not found — install glib-networking on the build host"; exit 1; }
  mkdir -p "$APPDIR/usr/lib/gio/modules"
  cp -L "$TLSMOD" "$APPDIR/usr/lib/gio/modules/libgiognutls.so"
  echo "==> bundled the gio TLS module from $TLSMOD"
  bundle_deps_of "$TLSMOD"
fi

# 1c. Drop libwayland-client. It has to be the host's copy, because the host's Mesa is linked
#     against it and we are first on LD_LIBRARY_PATH — see defect 3 in the header. Everything the
#     bundle needs from it exists in 1.22, and every host that can open a window ships a newer one.
for lib in "$APPDIR"/usr/lib/libwayland-client.so.0*; do
  [ -e "$lib" ] || continue
  rm -f "$lib"
  echo "==> removed $(basename "$lib") — the host's Mesa must link its own"
done

# 1d. GStreamer plugins, so the webview can decode a music video. See defect 5 in the header.
#     Named one by one rather than "everything in the host's plugin directory" or "everything in
#     these packages": gstreamer1.0-plugins-good alone is 74 plugins, and copying the lot drags
#     libv4l2, libdv, libshout, libwavpack and a camera stack into the bundle for a feature that
#     plays one muted video-only VP9 stream. The set below is that pipeline, plus mp4 and the audio
#     decoders in case a stream ever carries sound, plus an audio sink because WebKit builds one
#     either way. If WebKit turns out to want something else, the element check in
#     scripts/appdir-foreign-check.sh is what will say so.
GST_PLUGINS="libgstcoreelements.so libgsttypefindfunctions.so libgstplayback.so libgstapp.so
             libgstaudioconvert.so libgstaudioresample.so libgstvolume.so
             libgstvideoconvertscale.so libgstmatroska.so libgstisomp4.so libgstvpx.so
             libgstopus.so libgstvorbis.so libgstogg.so libgstaudioparsers.so
             libgstautodetect.so libgstalsa.so libgstpulseaudio.so libgstopengl.so libgstlibav.so"
GSTDIR="$APPDIR/usr/lib/gstreamer-1.0"
if ls "$GSTDIR"/libgst*.so >/dev/null 2>&1; then
  echo "==> GStreamer plugins already bundled"
else
  mkdir -p "$GSTDIR"
  for plugin in $GST_PLUGINS; do
    src=""
    for dir in /usr/lib/x86_64-linux-gnu/gstreamer-1.0 /usr/lib64/gstreamer-1.0 /usr/lib/gstreamer-1.0; do
      [ -e "$dir/$plugin" ] && { src="$dir/$plugin"; break; }
    done
    [ -n "$src" ] || {
      echo "$plugin not on the build host: install the gstreamer1.0-plugins-base/-good/-gl/-libav/-alsa set"
      exit 1; }
    cp -L "$src" "$GSTDIR/$plugin"
    # Everything linuxdeploy bundles gets RUNPATH $ORIGIN; ours arrive after it ran, so give them
    # the same treatment. Their dependencies sit one directory up, in usr/lib. AppRun's
    # LD_LIBRARY_PATH would find them anyway, so a host without patchelf is not fatal, but a plugin
    # that resolves on its own is a plugin that still works wherever GStreamer dlopens it from.
    patchelf --set-rpath '$ORIGIN/..' "$GSTDIR/$plugin" 2>/dev/null || true
    bundle_deps_of "$src"
  done
fi

# The scanner is a separate binary GStreamer forks to probe plugins out of process. Without it the
# probe happens in-process instead, which works right up until one bad plugin takes the whole web
# process down with it.
if [ -e "$APPDIR/usr/bin/gst-plugin-scanner" ]; then
  echo "==> gst-plugin-scanner already bundled"
else
  SCANNER=""
  for cand in /usr/lib/x86_64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner \
              /usr/libexec/gstreamer-1.0/gst-plugin-scanner \
              /usr/lib64/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner; do
    [ -e "$cand" ] && { SCANNER="$cand"; break; }
  done
  [ -n "$SCANNER" ] || { echo "gst-plugin-scanner not found on the build host"; exit 1; }
  cp -L "$SCANNER" "$APPDIR/usr/bin/gst-plugin-scanner"
  patchelf --set-rpath '$ORIGIN/../lib' "$APPDIR/usr/bin/gst-plugin-scanner" 2>/dev/null || true
  echo "==> bundled gst-plugin-scanner from $SCANNER"
  bundle_deps_of "$SCANNER"
fi

# 1e. Move the gnutls stack off the library path. It has to be the host's, because Ubuntu's gnutls
#     hardcodes /etc/ssl/certs/ca-certificates.crt as its trust store and openSUSE has no such file
#     — see defect 6 in the header. LAST of the bundling steps on purpose: everything above calls
#     bundle_deps_of, and although HOST_BASELINE now stops it copying these back, moving them after
#     the fact is what makes that true no matter which order the steps end up in.
#
#     gnutls alone is not enough. A soname is loaded once per process, so the host's gnutls resolves
#     libnettle.so.8 to ours whenever ours is on the path, and 3.8.10+ wants
#     nettle_rsa_oaep_sha384_decrypt, which our nettle 3.9 does not have. The whole chain moves or
#     none of it does.
#
#     Moved rather than deleted, because which copy wins is a runtime decision (2a). libavformat
#     DT_NEEDs libgnutls and libarchive DT_NEEDs libnettle.so.8, so both are load-time dependencies
#     of the app itself: a host that cannot serve the whole chain would go from "sign-in is broken"
#     to "does not start", which is worse than the bug being fixed. Arch is exactly that host, its
#     nettle 4.0 is libnettle.so.9. So the copies sit in a subdirectory the loader never looks in
#     and 2a puts it back on the path when the host comes up short.
#
#     Measured against the shipped 0.6.8 AppDir in containers, trust anchors reachable by the
#     webview's TLS stack, before -> after: openSUSE 0 -> 435, Debian 121 -> 121, Fedora 388 -> 388,
#     Arch 164 -> 119 (ours, via 2a), Ubuntu 121 -> 121. On openSUSE a real glib-networking
#     handshake to music.youtube.com went from "Unacceptable TLS certificate", the string in #164,
#     to OK. appdir-foreign-check.sh counts those anchors on every build now.
GNUTLS_STACK="libgnutls.so.30 libnettle.so.8 libhogweed.so.6 libgmp.so.10 libtasn1.so.6
              libp11-kit.so.0 libidn2.so.0 libunistring.so.5"
FALLBACK="$APPDIR/usr/lib/gnutls-fallback"
mkdir -p "$FALLBACK"
for soname in $GNUTLS_STACK; do
  for lib in "$APPDIR"/usr/lib/"$soname"* "$APPDIR"/usr/lib64/"$soname"*; do
    [ -e "$lib" ] || continue
    mv -f "$lib" "$FALLBACK/$(basename "$lib")"
    echo "==> moved $(basename "$lib") into gnutls-fallback/"
  done
done
# p11-kit's module directory goes with it: nothing can load those without libp11-kit.so.0 anyway.
rm -rf "$APPDIR/usr/lib/pkcs11" "$APPDIR/usr/lib64/pkcs11"
[ -e "$FALLBACK/libgnutls.so.30" ] || {
  echo "no libgnutls.so.30 in the AppDir at all — did linuxdeploy stop bundling it?"; exit 1; }

# 2. Point GIO_EXTRA_MODULES at the AppDir's own module directories — never the host's, see the
#    header. Appended rather than edited in place: AppRun *sources* the hook, so the last assignment
#    wins, and appending can't be broken by linuxdeploy reshaping the lines above it.
HOOK="$APPDIR/apprun-hooks/linuxdeploy-plugin-gtk.sh"
[ -f "$HOOK" ] || { echo "no GTK apprun hook at $HOOK — did linuxdeploy's plugin layout change?"; exit 1; }
echo "==> Overriding GIO_EXTRA_MODULES with the bundled module dirs…"
# The sentinel is the newest thing this block writes, not the first: grepping for the comment would
# make the script decide it had already run over an AppDir fixed by an older copy of itself, and
# silently leave half the fix out.
grep -q 'GIO_MODULE_DIR' "$HOOK" || cat >> "$HOOK" <<'EOF'

# Limusic: the value written above is unusable — it contains a literal newline and an absolute path
# into the build machine's target/ dir. Bundled dirs only: host modules are built against the host's
# GLib and fail to load into ours (gvfs wants g_variant_builder_init_static, GLib >= 2.84).
export GIO_EXTRA_MODULES="$APPDIR/usr/lib/gio/modules:$APPDIR/usr/lib64/gio/modules"
# GIO_EXTRA_MODULES only *adds* a directory. GLib still scans the one compiled into it, which for an
# Ubuntu build is /usr/lib/x86_64-linux-gnu/gio/modules — the same path the user's Debian or Ubuntu
# host keeps its own gvfs modules in, built against a GLib far newer than the one we ship. GLib
# loads them, they fail, two lines of "undefined symbol: g_variant_builder_init_static" per process.
# GIO_MODULE_DIR replaces that compiled-in path instead of adding to it.
export GIO_MODULE_DIR="$APPDIR/usr/lib/gio/modules"
EOF

# …and make sure there is actually something there to load. An AppDir with no TLS module is the
# original bug in a new hat: the app starts, looks fine, and can't reach YouTube.
# Counted with an explicit loop, not `ls dirA/glob dirB/glob`: ls exits nonzero when *either* path
# is missing, even after listing the other one fine. Ubuntu AppDirs have no usr/lib64, so that
# spelling reported "no modules" while the module sat right there. It cost a release number.
MODS=0
for d in "$APPDIR/usr/lib/gio/modules" "$APPDIR/usr/lib64/gio/modules"; do
  [ -d "$d" ] || continue
  for f in "$d"/libgio*.so; do [ -e "$f" ] && MODS=$((MODS + 1)); done
done
[ "$MODS" -gt 0 ] || {
  echo "no gio modules bundled — install glib-networking on the build host, or the webview gets no TLS backend"
  exit 1; }
echo "==> bundled gio modules: $MODS"

# 2a. The gnutls fallback from 1e, wired up. Sonames only, no `ldconfig`: that lives in /usr/sbin,
#     which is not on every user's PATH. AppRun.wrapped appends the existing LD_LIBRARY_PATH after
#     its own AppDir entries, so what we export here is searched last, which is exactly right for a
#     fallback.
grep -q 'gnutls-fallback' "$HOOK" || cat >> "$HOOK" <<'EOF'

# Limusic: the gnutls stack is deliberately off the library path (defect 6). Ubuntu's gnutls
# hardcodes /etc/ssl/certs/ca-certificates.crt as its trust store, so on a host that keeps its CA
# bundle anywhere else (openSUSE) ours leaves the webview trusting nothing. The host's own gnutls
# knows where the host's trust store is.
#
# That only works when the host can serve the whole chain, because a soname is loaded once per
# process: the host's gnutls resolves libnettle.so.8 to ours if ours is on the path, and 3.8.10+
# wants a symbol our nettle 3.9 does not have. So it is all of the host's or all of ours. Arch is
# the "all of ours" case — nettle 4.0 there is libnettle.so.9, which cannot serve the bundled
# libarchive and libsrt, so we put our own copies back and accept the compiled-in trust path, which
# is the file Arch actually has. Whatever this picks, the check in scripts/appdir-foreign-check.sh
# counts the trust anchors that come out of it.
_gnutls_host=1
for _so in libgnutls.so.30 libnettle.so.8 libhogweed.so.6; do
  _found=""
  for _d in /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib /lib64 /lib/x86_64-linux-gnu; do
    [ -e "$_d/$_so" ] && { _found=1; break; }
  done
  [ -n "$_found" ] || _gnutls_host=""
done
[ -n "$_gnutls_host" ] || export LD_LIBRARY_PATH="$APPDIR/usr/lib/gnutls-fallback${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
unset _gnutls_host _so _found _d
EOF

# 2b. Point GStreamer at the bundled plugins. Same shape as GIO_MODULE_DIR above and for the same
#     reason: GST_PLUGIN_SYSTEM_PATH_1_0 *replaces* the path compiled into libgstreamer, so the
#     host's plugin directory is never scanned. Its own sentinel, not defect 4's: an AppDir fixed by
#     an older copy of this script already has GIO_MODULE_DIR, and sharing a sentinel would make
#     this block silently skip itself there.
grep -q 'GST_PLUGIN_SYSTEM_PATH_1_0' "$HOOK" || cat >> "$HOOK" <<'EOF'

# Limusic: WebKit decodes <video> through GStreamer, and the plugins travel with us (defect 5).
# _1_0 is the versioned spelling GStreamer 1.x reads first, and SYSTEM_PATH replaces the compiled-in
# /usr/lib/x86_64-linux-gnu/gstreamer-1.0 rather than adding to it, so a Debian host's own plugins
# are never loaded into the older GStreamer we ship.
export GST_PLUGIN_SYSTEM_PATH_1_0="$APPDIR/usr/lib/gstreamer-1.0"
export GST_PLUGIN_SCANNER_1_0="$APPDIR/usr/bin/gst-plugin-scanner"
# The default registry is $XDG_CACHE_HOME/gstreamer-1.0/registry.<arch>.bin, shared with every other
# GStreamer app on the machine. Ours describes a different plugin set to a different GStreamer, so
# give it its own file rather than have the two rewrite each other's on every launch.
export GST_REGISTRY_1_0="${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/limusic/gstreamer-registry.bin"
mkdir -p "$(dirname "$GST_REGISTRY_1_0")" 2>/dev/null || true
EOF

# …and, exactly as with the gio modules, check there is something there to find. An AppDir whose
# plugin directory is empty is v0.5.0 again: the app starts, plays audio, and dies the moment a
# music video draws.
GSTPLUGINS=0
for f in "$GSTDIR"/libgst*.so; do [ -e "$f" ] && GSTPLUGINS=$((GSTPLUGINS + 1)); done
[ "$GSTPLUGINS" -gt 0 ] || {
  echo "no GStreamer plugins bundled: the webview would abort on the first music video"
  exit 1; }
echo "==> bundled GStreamer plugins: $GSTPLUGINS"

# 3. Repack with the packer Tauri already downloaded for the original bundle.
# Globbed, not hardcoded: the exact filename is Tauri's business and CI runs a different CLI version.
PACKER="$(ls "$HOME"/.cache/tauri/linuxdeploy-plugin-appimage*.AppImage 2>/dev/null | head -1 || true)"
[ -n "$PACKER" ] && [ -x "$PACKER" ] || {
  echo "no linuxdeploy appimage plugin in $HOME/.cache/tauri — did \`tauri build --bundles appimage\` run?"; exit 1; }
echo "==> Repacking $(basename "$APPIMAGE")…"
rm -f "$APPIMAGE"
# NO_STRIP: linuxdeploy ships an ancient `strip` that chokes on modern ELF sections (DT_RELR).
# APPIMAGE_EXTRACT_AND_RUN: the packer is itself an AppImage and CI runners have no FUSE.
ARCH=x86_64 NO_STRIP=true OUTPUT="$APPIMAGE" APPIMAGE_EXTRACT_AND_RUN=1 \
  "$PACKER" --appdir "$APPDIR"
[ -f "$APPIMAGE" ] || { echo "packer produced no $APPIMAGE"; exit 1; }
chmod +x "$APPIMAGE"

# 4. Re-sign: repacking invalidated the signature the bundler made, and latest.json is generated
#    from the .sig — a stale one silently breaks every self-update.
if [ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
  echo "==> Re-signing…"
  rm -f "$APPIMAGE.sig"
  if command -v tauri >/dev/null; then tauri signer sign "$APPIMAGE"; else cargo tauri signer sign "$APPIMAGE"; fi
  [ "$APPIMAGE.sig" -nt "$APPIMAGE" ] || { echo "no fresh .sig next to $APPIMAGE after signing"; exit 1; }
  echo "==> Signed: $APPIMAGE.sig"
else
  # Local test runs don't need a valid signature. Every path that ships one checks for the key first.
  echo "==> TAURI_SIGNING_PRIVATE_KEY not set — skipping the re-sign (unsignable AppImage, test only)"
  rm -f "$APPIMAGE.sig"
fi

echo "==> Done: $APPIMAGE"
