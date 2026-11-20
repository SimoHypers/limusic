# UI performance: why it is hard

`UI-PERFORMANCE.md` holds the rules. This file holds what it was like to follow them: what we
wanted to build, what the engine charged for it, what we settled for, and what wasted time on the
way. The rules say what to do; this says why a change that looks finished in an hour takes a day,
so the next person budgets for it instead of being surprised.

Add an entry whenever UI work runs into the engines: an effect cut or reshaped because of what it
cost, a measurement that contradicted the theory, a tool that lied or broke. Newest at the bottom
of the log, dated, with the numbers.

## Why we can't skip it

- **The app is open all day and something is always moving.** Music plays for hours with the
  player bar ticking four times a second and lyrics sweeping every frame. An effect is never paid
  once, it is paid for every frame of every song, on a laptop's battery and fans. A lyrics tab that
  costs 70% of a core instead of 40% is a hot laptop, not a dropped frame.
- **The people who report lag are not on our hardware.** Development happens on a desktop where
  WebKitGTK at full speed hides most costs. Reports come from integrated graphics, old CPUs and the
  distros people keep alive on old machines. 1.0.0 shipped hover zooms that were free here and made
  the app unusable on Windows (#341). A root custom property write froze the window for 200 ms a
  time (#217).
- **A bad release installs itself.** The updater puts a regression on tens of thousands of machines
  before the first report, so a slow effect has to be caught before the tag, not after.
- **Three engines price the same CSS differently.** WebKitGTK on Linux, WebView2 (Chromium) on
  Windows, WKWebView on macOS. Cheap on one is the lag report on another, and only two of the three
  can be measured on the development machine.

## Why it is hard

- **WebKitGTK is the slowest engine we ship and the one we develop on.** It does style and layout
  about six times slower than Chromium on our DOM, and it does more of its work on the main thread.
  Most motion that reads as polished elsewhere was designed for Chromium. Better Lyrics, the
  reference for lyrics, is a Chromium extension.
- **The effects that make motion feel premium are the expensive ones.** Moving or filtering
  something that also repaints every frame is what WebKitGTK charges most for, and a karaoke sweep
  repaints every frame by design. A glow or a lift on each sung word is exactly that.
- **Intuition is wrong about as often as it is right.** Moving ten whole lines with a `translate`
  animation measured as free; moving one word two pixels cost ten points of a core. The obvious
  fixes for that (write it per frame instead of as a transition, give the words their layers up
  front) changed nothing. Only switching things off and measuring again settles anything, and each
  of those runs costs minutes.
- **Looking at it doesn't show the problem.** A WebKitGTK snapshot is a software repaint, so a
  glitch between composited layers never appears in one. Playwright's WebKit is not the system
  WebKitGTK. Absolute numbers from any harness are not the app's; only before and after on the
  same machine in the same session compare.
- **Correct in theory, wrong on screen.** An effect can be valid CSS, cheap, and still render
  differently per engine, like a filter over clipped text glowing as a box.
- **Linux adds a cost no page can remove.** GTK3 presents the whole window on the UI process's
  main thread, so anything animating at sixty frames a second costs 7 to 20% of a core before the
  page does any work. There is no free animation on Linux, only cheaper ones.
- **The measurements drift.** Builds, the owner's own app, a browser or a shader compile in the
  background move the baseline by 15 to 25%. A comparison only counts when before and after run
  interleaved, in the same round.

## Log

### 2026-11-14, lyrics motion (`feat/lyrics-motion`)

**Wanted:** Better Lyrics' feel in every lyrics view: lines gliding in a cascade, the active line
growing, a soft sweep, every sung word lifting and glowing, held notes cresting letter by letter.

**What it cost** (system WebKitGTK 2.52.1, `framecost.py`, lyrics tab open, web process as % of
one core):

- Glow on every word, as a `drop-shadow` filter: 56% to 70%, frames over 20 ms 3% to 11%. Each
  sung word repaints under its filter every frame of the sweep. Cut; only held notes glow.
- Lifting each word two pixels as it is sung: 40% to 49% as a CSS transition, 46% to 57% written
  per frame, and creating the words' layers up front first changed nothing. The motion itself is
  the cost. Cut for normal words; held notes still crest.
- The cascade (a `translate` animation on every line on screen), the line scale, the soft sweep and
  the cover line's blur handoff: each within noise.
- The held-note glow as a filter drew a box around the letters, the whole background the sweep is
  clipped from, not the glyphs. Rebuilt as a clear copy of the letters under a fixed `text-shadow`,
  faded by opacity: letter-shaped, and 45% to 49% on a lyric where every line ends on a held note.
- End result against `dev`: 37% to 49% on that worst case, frame times unchanged.

**What wasted time:**

- The old active line had `scale-[1.04]` with `transition-[color,opacity,transform]`. In Tailwind
  v4 `scale` is its own property, so it never animated, it snapped. Nothing looked broken in the
  code.
- zsh does not split `$args`, so three ablation runs got one malformed argument and printed
  nothing. It read like the harness had failed.
- The same baseline build measured 43% in the first round and 37% later, because the builds had
  just finished. Every comparison had to be rerun interleaved.
- A GTK `OffscreenWindow` has no GL context on this machine, so a quick WebKitGTK render check
  dumped core; it needs a real window.
- The fake lyrics in the harness had no held notes, so the most expensive path had to be fed in
  through a `--js` override before it could be measured at all.

### 2026-11-15, the AppImage on NVIDIA paints without the GPU

**Wanted:** the AppImage to feel the way `cargo tauri dev` does. Every animation was tuned in dev,
and the AppImage, which most Linux users run, felt clearly worse, transitions most of all.

**What we found:** the AppImage bundles Ubuntu's WebKitGTK, and Ubuntu builds it with Debian's
`disable-nvidia-dmabuf.patch` (WebKit bug 262607, Debian #1039720; openSUSE and Gentoo carry it
too). When the GL vendor is NVIDIA it turns the DMABUF renderer off, and hardware acceleration with
it: `webkit://gpu` reports `Policy: never`, and the web process has no compositor thread. The
renderer defaults in `run()` for NVIDIA (`WEBKIT_DMABUF_RENDERER_FORCE_SHM`,
`WEBKIT_SKIA_ENABLE_CPU_RENDERING`) never take effect there; the comments and the table in
`UI-PERFORMANCE.md` had assumed the AppImage was on the SHM path. `WEBKIT_FORCE_DMABUF_RENDERER=1`
switches acceleration back on. Fedora's and Arch's WebKitGTK don't carry the patch, which is why
dev never showed any of this. 1.4.0-rc.7 and the current `dev` bundle the same WebKitGTK, GTK and
hook byte for byte.

**What it costs** (`framecost.py`, the same production build of the SPA in every leg, GTX 1060 on
driver 580.178.04, KDE Plasma on Wayland, a 1300x720 transparent window on a 75 Hz screen, which
WebKit paces at 60 in every leg; system WebKitGTK 2.52.1 against the bundled 2.52.6; median of two
runs; CPU is UI process plus web process plus Xwayland as % of one core):

| | navigation p95 | opening the player, frames over 20 ms | lyrics | hover | scroll |
|---|---|---|---|---|---|
| dev, Wayland, CPU painting (current `dev`) | 34 ms | 6% | 41% | 42% | 35% |
| dev, Wayland, GPU painting (before CPU painting) | 21 ms | 4% | 40% | 42% | 35% |
| system libraries on X11 + SHM, CPU painting | 28 ms | 6% | 54% | 56% | 49% |
| AppImage as shipped (software) | 40 ms | 14% | 36% | 31% | 26% |
| AppImage, `LIMUSIC_GDK_BACKEND=wayland` (software) | 39 ms | 14% | 45% | 43% | 36% |
| AppImage, acceleration forced, X11 + SHM, CPU painting | 38 ms | 6% | 55% | 57% | 50% |
| AppImage, acceleration forced, Wayland, CPU painting | 36 ms | 6% | 42% | 43% | 35% |
| AppImage, acceleration forced, Wayland, GPU painting | 23 ms | 5% | 41% | 42% | 35% |

Taken one factor at a time:

- **Software painting costs the transitions.** Navigation p95 goes from 21-25 ms to 39-40 ms and
  the window presents 13-15 frames a second instead of 19-20 while it does; opening the player
  goes from 5-6% of frames over 20 ms to 14%; the lyrics sweep from 2% to 6%, p95 18 to 21 ms. A
  steady animation is cheaper in software, because nothing is composited or uploaded: hovering a
  shelf 31% of a core against 42-43%, scrolling 26% against 35%. Hence a build that measures
  cheaper and feels worse.
- **X11 with SHM, once accelerated, costs every steady animation 14-17 points:** hover 42% to
  57-59%, scroll 35% to 50-52%, lyrics 41% to 55-58%, 12-15 points of it in Xwayland. Frame times
  don't move.
- **CPU painting costs navigation**, on both builds: p95 21 to 34 ms on the system one, 23 to 36
  ms on the bundled one, with single runs reaching 41-42 ms; opening the player costs 6-7 points
  more. Hover, scroll and lyrics are unchanged. Twice the painting threads
  (`WEBKIT_SKIA_CPU_PAINTING_THREADS=6` on a six-core CPU, default three) bought nothing: navigation
  p95 38 against 34 ms on the bundled build, 29 against 32 on the system one, three runs each with
  ranges that overlap. It stays on, because the freeze it fixes came six times in two days.
- **The bundled build costs nothing of its own.** In the same mode it measures within 1-3 points
  and 2 ms of the system one. An earlier probe that put the AppImage at twice the web process CPU of
  dev was measuring the software path.
- **The Wayland backend alone buys nothing on NVIDIA.** The page is still painted in software, and
  presenting the transparent window costs the UI process 15 points more: hover 31% to 43%.

**What we settled for:** acceleration forced on Wayland, and the AppImage running on Wayland
wherever the session is Wayland (the GTK hook's `GDK_BACKEND=x11` is deleted when the AppImage is
built). That matches dev within 2 points and 3 ms in every scenario, and on the development
machine it felt clearly smoother. CPU painting stays on, so page changes keep the 34-36 ms p95 that
dev has now rather than the 21-23 ms it had before. An X11 session keeps the software path: forcing
acceleration there would cost every steady animation 25 points of a core.

**What wasted time:**

- The renderer had been assumed, not checked, since August. One look at `webkit://gpu` inside the
  bundled stack settled it; earlier AppImage measurements had explained the gap
  with the bundled build and with X11.
- zsh doesn't split `$var` into words, so the first loops over the legs passed one malformed
  argument each, and the inspection scripts quietly read nothing.
- A browser playing video during the first two of four rounds raised every number by about 10%;
  the table uses the two clean rounds, and the noisy ones keep the same order.
- A probe window opens on whichever screen holds the pointer, 60 Hz or 75 Hz. A KWin script
  loaded for the session only, and unloaded afterwards, pinned it to one place.
- With GPU painting, the web process segfaulted inside the NVIDIA driver as it shut down at the
  end of a run, destroying Skia's GL context: thirteen core dumps and crash notifications across
  both builds, every one at a run's last second, none with CPU painting, and none during a
  measurement. It reads like a new failure until the crash times are set against the legs.
