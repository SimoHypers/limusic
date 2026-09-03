#!/usr/bin/env bash
# Check a Limusic AppDir on a distro that isn't the build host. Runs INSIDE a container, with the
# AppDir mounted at /app, and installs the desktop packages it needs itself.
#
#   podman run --rm -v "$PWD/target/release/bundle/appimage/limusic.AppDir:/app:ro,z" \
#     -v "$PWD/scripts/appdir-foreign-check.sh:/check.sh:ro,z" debian:sid bash /check.sh
#
# Also runs against a published release: extract it with `--appimage-extract` and mount
# squashfs-root instead. CI calls it for debian:sid, archlinux and opensuse/tumbleweed on every
# build.
#
# Five checks, in the order they catch things:
#
#   1. ldd -r on the app binary — every load-time symbol resolves. Catches KI-6, KI-7, KI-8.
#   2. Host libraries the app dlopens are still loadable with the AppDir on the search path.
#      An AppDir is first on LD_LIBRARY_PATH, so anything the host dlopens later (Mesa's EGL
#      vendor, gio modules, pixbuf loaders, IM modules) links against *our* copies of the shared
#      sonames. If ours are older than what the host's copy needs, it fails to load and the caller
#      usually reports something unrelated-looking. That is KI-9, and ldd -r cannot see it.
#   3. The TLS stack the webview uses can actually reach the host's trust anchors. Ubuntu's gnutls
#      hardcodes /etc/ssl/certs/ca-certificates.crt, so bundling it left the webview with zero
#      anchors on openSUSE and every HTTPS load failed as "Unacceptable TLS certificate" (#164).
#      Nothing else here sees that: the app starts, plays audio, and only the webview is blind.
#
#   4. The bundled GStreamer can still produce the elements a music video needs. WebKit decodes
#      <video> through GStreamer, and v0.5.0 shipped the GStreamer core libraries with no plugin
#      directory at all: the registry came up empty on every distro whose plugin path is not
#      Debian's, and WebKit aborted the web process the moment a video drew. Nothing else here sees
#      that, because the app starts and plays audio perfectly without a single plugin.
#   5. An actual launch under Xvfb. The verification hole behind four broken releases in one night
#      was that nothing ever started the app anywhere except the build host's own OS family.
set -uo pipefail

APPDIR=/app
BIN="$APPDIR/usr/bin/limusic-app"
FAIL=0
step() { printf '\n── %s\n' "$1"; }
bad()  { echo "   FAIL: $1"; FAIL=1; }

step "installing a desktop package set"
if command -v apt-get >/dev/null; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update -qq
  apt-get install -y -qq --no-install-recommends \
    xvfb xauth dbus dbus-x11 python3 \
    libegl-mesa0 libgl1-mesa-dri libglx-mesa0 libgles2 libgl1 libegl1 \
    libharfbuzz0b libharfbuzz-icu0 libkrb5-3 libgssapi-krb5-2 libpango-1.0-0 \
    libasound2t64 libfribidi0 libusb-1.0-0 libcom-err2 libgpg-error0 libexpat1 \
    libfontconfig1 fonts-dejavu-core ca-certificates gvfs
elif command -v pacman >/dev/null; then
  pacman -Sy --noconfirm --quiet \
    xorg-server-xvfb xorg-xauth dbus python \
    mesa libglvnd harfbuzz harfbuzz-icu krb5 pango alsa-lib fribidi libusb \
    expat fontconfig ttf-dejavu ca-certificates gvfs gnutls
elif command -v dnf >/dev/null; then
  dnf install -y -q \
    xorg-x11-server-Xvfb xorg-x11-xauth dbus-daemon dbus-x11 python3 \
    mesa-libEGL mesa-libGL mesa-libGLES libglvnd harfbuzz krb5-libs pango alsa-lib \
    fribidi libusb1 expat fontconfig dejavu-sans-fonts ca-certificates gvfs
elif command -v zypper >/dev/null; then
  # openSUSE spells half of these differently: xvfb-run is its own package, dbus-run-session comes
  # from dbus-1-daemon, and there is no dbus-1-x11 at all.
  zypper -q --non-interactive in -y \
    xvfb-run xauth dbus-1-daemon python3 \
    Mesa-libEGL1 Mesa-libGL1 Mesa-libGLESv2-2 libglvnd harfbuzz-tools libharfbuzz0 krb5 \
    libpango-1_0-0 alsa-lib libfribidi0 libusb-1_0-0 libcom_err2 libgpg-error0 libexpat1 \
    fontconfig dejavu-fonts ca-certificates gvfs libgnutls30
else
  echo "   unknown package manager"; exit 1
fi
# gvfs is deliberately installed: it is what makes a host/bundle GLib mismatch visible. gnutls is
# installed for the same reason it is not bundled (defect 6): it is a host library now, and these
# images are barer than any desktop that would run the app. Debian and Fedora pull it in with gvfs;
# Arch and openSUSE have to be told.
# A half-installed container makes every check below meaningless, and the missing symbols it
# produces look exactly like a real defect (KI-8). Stop here instead.
for tool in xvfb-run dbus-run-session ldd python3; do
  command -v "$tool" >/dev/null || { echo "   package install failed: no $tool"; exit 1; }
done

# The library path the app actually runs with. AppRun.wrapped prepends the AppDir's own lib dirs
# and keeps whatever the GTK hook exported after them, and since #164 that hook adds the gnutls
# fallback directory on a host that cannot serve libgnutls/libnettle/libhogweed itself. Computed in
# a subshell so the hook's other exports (XDG_DATA_DIRS, GST_*) do not leak into the probes below,
# which run their own copies deliberately.
APPDIR_LD="$(set +u
  export APPDIR="$APPDIR"
  . "$APPDIR/apprun-hooks/linuxdeploy-plugin-gtk.sh" >/dev/null 2>&1
  echo "$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}")"

step "ldd -r: every load-time symbol resolves"
echo "   library path: $APPDIR_LD"
LD_LIBRARY_PATH="$APPDIR_LD" ldd -r "$BIN" 2>&1 \
  | grep -E 'not found|undefined symbol' | sort -u > /tmp/ldd.txt
if [ -s /tmp/ldd.txt ]; then sed 's/^/   /' /tmp/ldd.txt; bad "unresolved symbols in $BIN"
else echo "   clean"; fi

step "host libraries the app dlopens still load with the AppDir on the path"
# The graphics stack only. Every other plugin directory the app touches is redirected to the AppDir
# by AppRun (GIO_MODULE_DIR, GDK_PIXBUF_MODULE_FILE, GTK_IM_MODULE_FILE, GTK_PATH), so the host's
# copies are never loaded and probing them only produces false positives; the launch check below
# greps for GLib's "Failed to load module:" and covers them for real. Nothing redirects libglvnd,
# which must find the host's driver because only the host's kernel and Mesa agree on it.
# Only host files matter here: anything whose soname we also bundle is resolved to *our* copy at
# runtime, so the host's version of it is never loaded and a failure on it means nothing.
BROKEN=0
for d in /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib; do
  [ -d "$d" ] || continue
  for f in "$d"/libEGL_*.so.[0-9] "$d"/libGLX_*.so.[0-9] "$d"/libgbm.so.[0-9] "$d"/dri/*.so; do
    [ -f "$f" ] || continue
    [ -e "$APPDIR/usr/lib/$(basename "$f")" ] && continue
    with=$(LD_LIBRARY_PATH="$APPDIR_LD" ldd -r "$f" 2>&1 | grep -E 'undefined symbol|not found' | sort -u)
    [ -n "$with" ] || continue
    without=$(ldd -r "$f" 2>&1 | grep -E 'undefined symbol|not found' | sort -u)
    [ "$with" = "$without" ] && continue   # already broken on its own; not something we caused
    BROKEN=1
    echo "   $f"
    comm -13 <(echo "$without") <(echo "$with") | sed 's/^/      /' | head -5
    ldd "$f" 2>/dev/null | awk '/=> \//{print $1}' | while read -r s; do
      [ -e "$APPDIR/usr/lib/$s" ] && echo "      ^ we shadow: $s"
    done | sort -u
  done
done
if [ "$BROKEN" = 1 ]; then
  bad "the AppDir breaks host libraries the app loads at runtime — drop the shadowing library from the AppDir in scripts/fix-appdir-tls.sh"
else
  echo "   clean"
fi

step "the TLS stack can reach this host's trust anchors"
# Loads whichever libgnutls this host resolves to under the app's own library path — the host's
# normally, the bundled fallback where fix-appdir-tls.sh's 2a decided the host can't serve the chain
# — and asks it to populate a trust list from the system store, which is exactly what
# glib-networking's gnutls backend does for every https:// the webview opens. Zero anchors is a
# webview that trusts nothing, and every other check here passes right through it.
cat > /tmp/trustprobe.py <<'PROBE'
import ctypes, sys
g = ctypes.CDLL("libgnutls.so.30")
tl = ctypes.c_void_p()
if g.gnutls_x509_trust_list_init(ctypes.byref(tl), 0) != 0:
    print("   could not init a gnutls trust list"); sys.exit(1)
n = g.gnutls_x509_trust_list_add_system_trust(tl, 0, 0)
print(f"   system trust anchors: {n}")
sys.exit(0 if n > 0 else 1)
PROBE
LD_LIBRARY_PATH="$APPDIR_LD" python3 /tmp/trustprobe.py \
  || bad "the webview's TLS stack finds no trust anchors on this host — every https:// load fails as \"Unacceptable TLS certificate\""

step "the bundled GStreamer can decode a music video"
# Sources the AppRun hook rather than setting GST_PLUGIN_SYSTEM_PATH_1_0 by hand, because half of
# what broke in v0.5.0 was that nothing set it at all. Loads our libgstreamer by absolute path and
# leaves LD_LIBRARY_PATH alone: the bundled libs carry RUNPATH $ORIGIN, so they resolve on their
# own, and putting the AppDir on python's own library path would shadow its libffi for no reason.
cat > /tmp/gstprobe.py <<'PROBE'
import ctypes, sys
gst = ctypes.CDLL("/app/usr/lib/libgstreamer-1.0.so.0")
gst.gst_init(None, None)
gst.gst_element_factory_find.restype = ctypes.c_void_p
# A muted, video-only VP9/WebM stream, plus the audio sink WebKit builds whether or not the
# stream has any audio in it.
need = ["typefind", "matroskademux", "vp9dec", "videoconvert", "videoscale", "audioconvert",
        "audioresample", "queue2", "appsink", "decodebin", "autoaudiosink"]
missing = [e for e in need if not gst.gst_element_factory_find(e.encode())]
print("   elements missing: " + (", ".join(missing) if missing else "none"))
sys.exit(1 if missing else 0)
PROBE
(
  # set +u for the hook only: it appends to XDG_DATA_DIRS, which a container does not set, and this
  # script's own `set -u` would kill the subshell on that line with the output redirected away.
  set +u
  export APPDIR="$APPDIR" HOME=/tmp/apphome
  mkdir -p "$HOME"
  . "$APPDIR/apprun-hooks/linuxdeploy-plugin-gtk.sh" >/dev/null 2>&1
  echo "   GST_PLUGIN_SYSTEM_PATH_1_0=${GST_PLUGIN_SYSTEM_PATH_1_0:-<unset>}"
  python3 /tmp/gstprobe.py
) || bad "the bundled GStreamer cannot build a video pipeline, so the webview aborts on a music video"

WEBVIEW_OK='webview bridge OK|webview Mozilla/5\.0'
step "launching the app under Xvfb"
export HOME=/tmp/apphome
mkdir -p "$HOME"
: > /tmp/run.log
timeout 90 dbus-run-session -- xvfb-run -a -s "-screen 0 1280x800x24" "$APPDIR/AppRun" \
  > /tmp/run.log 2>&1 &
RUNPID=$!
# Stop as soon as a webview works (about three seconds) or the app dies. The app never exits on its
# own, so without this the step would always burn the full timeout.
for _ in $(seq 90); do
  grep -qE "$WEBVIEW_OK" /tmp/run.log && break
  kill -0 "$RUNPID" 2>/dev/null || break
  sleep 1
done
kill "$RUNPID" 2>/dev/null
wait "$RUNPID" 2>/dev/null
# The app is killed rather than exiting, so its status says nothing; the log is the verdict.
grep -viE 'dbind|StatusNotifier|libEGL warning|DRI3' /tmp/run.log | head -40 | sed 's/^/   /'
if grep -qE 'Could not create .*EGL display|undefined symbol|cannot open shared object file|Failed to load module|webview never became ready|symbol lookup error|core dumped' /tmp/run.log; then
  bad "startup log contains a loader or webview failure (see above)"
fi
# Either line means a WebKit web process came up and round-tripped JS, which is what separated a
# working AppImage from the v0.2.14 one that logged everything else identically. "webview bridge OK"
# is a hidden harness webview; the UA line is the main window's SPA reporting navigator.userAgent
# back over IPC, so it proves the same thing about the window a user actually sees. Both are needed:
# since 0.6.7 the cipher webview is built on demand, so a startup that plays nothing never has one.
grep -qE "$WEBVIEW_OK" /tmp/run.log || bad "no webview ever became usable"

printf '\n'
[ "$FAIL" = 0 ] && { echo "PASS: $(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown}")"; exit 0; }
echo "FAIL: $(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown}")"
exit 1
