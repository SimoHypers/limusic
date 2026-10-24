# UI performance

Rules for any visual effect in `ui/`: blur, shadow, transition, animation, `will-change`,
`backdrop-filter`, a `requestAnimationFrame` loop, or a long list. Each one below was measured after
a lag report, and each cost real users frames before it was written down.

## Price every effect against a page that is always repainting

A screenshot of the app is static; the running app never is. While a song plays the player bar
repaints four times a second (position ticks), the lyrics sweep repaints every frame, and a pointer
moving across a shelf hovers a new card every few frames. An effect costs whatever it adds to *those*
repaints, multiplied by how often they happen.

The app also ships on three engines that price the same CSS differently:

| Platform | Webview | Engine |
|---|---|---|
| Linux | WebKitGTK | WebKit |
| Windows | WebView2 | Chromium |
| macOS | WKWebView | WebKit |

Something cheap on one can be the lag report on another. Lag reports come from laptops with
integrated graphics, not from the machine the effect was written on.

## Rules

**Hover animates colour.** `transition-colors` on hover is always fine. A transform animation
(`scale`, `translate`, `rotate`) on hover belongs only on a single element the pointer is directly
on, like one play button (`hover:scale-105`). On anything repeated (cards, rows, sidebar items, a
`group-hover:` target) the pointer starts one per item it crosses, and on Chromium each start and
end re-layerizes the whole page. Removing the card zoom, the stacked-sheet lift and the sidebar icon
zoom, together with the `.art-wash` change below, took Home from 46-52% of frames over 20 ms to 7-9%
(#341). Cards may keep a `transition-opacity` fade.

**Rows in a scrolling list carry no transition at all.** Reveal a row's buttons with `invisible
group-hover:visible group-focus-within:visible`. Rows stream under a stationary pointer while
scrolling, so every row takes and drops `:hover`: the row's `transition-colors` and its faded
controls were each about 30% of the queue's dropped frames, and together about 90% (#311,
`TrackRow.svelte`).

**Blur a small source, once.** A blurred artwork backdrop uses a 96px thumbnail
(`thumb(url, 96)`: a 40px blur discards everything finer). The image and the gradient that fades
it out sit together in one clipped wrapper that carries the `.art-wash` class; promoted separately,
the two layers land a pixel apart while scrolling and a line of raw wash flickers at the fade's
edge (`HomeHero.svelte`). `.art-wash` puts that wrapper on its own layer on WebKit only, because
the same promotion made the lyrics panel drop every frame on Chromium (#341, `layout.css`). A
full-screen blur is baked into a small canvas once per track and upscaled (`artroom.svelte.ts`,
the backdrop of theater mode and the player view).

**`backdrop-filter` goes on small elements over still content.** A backdrop filter re-runs whenever
anything under it repaints. Over the whole window that means four times a second while music plays,
which is why dialog overlays are plain `bg-black/80` (#341: Settings, idle with a song playing, from
15% of frames over 20 ms to 0%). A sticky header over scrolling content turns opaque when pinned (`routes/+page.svelte`).

**Shadows on repeated cards fade in as a layer.** Cards have no resting shadow, and the hover shadow
is a pre-blurred element whose opacity changes (`MediaCard.svelte`). WebKit rasterizes in tiles and
re-blurs every shadow in a tile when one card in it repaints; a resting `shadow-sm` was three
quarters of the hover cost.

**Per-track and per-tick writes land on a leaf element.** Writing a custom property on `<html>`
restyles the whole document: 160-200 ms and about 10 MB that WebKitGTK never gives back, per write
(#217). Transitioning a registered (`@property`) custom property does that once a frame. Keep the
artwork tint to the fills listed in `layout.css`.

**A control that sets a root token previews on a leaf and commits on release.** A slider or colour
picker writing `<html>` per pointer move queues one whole-document restyle per move. The old
Appearance tint slider took 5.6-8.2 s of main thread to play out a 0.75 s drag on WebKitGTK 2.52
(~120 ms a frame). Now a drag only restyles `ThemeMock`, which carries its own `theme-<id>` class and
inline overrides, and `<html>` is written once on `onValueCommit`: the same drag runs at 16 ms frames
with nothing blocked. Keep the live preview to one miniature: seven gallery miniatures following
the drag cost 60 of 150 frames at 4x CPU on Chromium, the single preview none
(`AppearanceSettings.svelte`).

**A `requestAnimationFrame` loop runs only while its output is on screen and changing.** Gate it the
way `LyricsView.svelte` gates the karaoke clock (`needsFrameClock`), so the app reaches idle frames.

**Layout is read on demand, never at mount.** Reading `scrollWidth`, `clientWidth`,
`getBoundingClientRect()` and friends right after content is inserted forces the engine to lay out
everything just mounted, synchronously, before `content-visibility: auto` has skipped anything
off-screen. Measure when the value is needed: on pointer enter, on scroll, on resize. `Shelf`'s
arrow check ran in a mount `$effect` and made going back to Home take ~870 ms on WebKitGTK instead
of ~250 ms, 640 ms of it in that one read (`Shelf.svelte`).

**Long lists are windowed.** Use `rows.svelte.ts` for anything that can reach hundreds of rows.
`content-visibility: auto` pays off only on lists that genuinely mount thousands of rows
(`TrackRow`'s `lazy` prop); on a windowed or short list it costs more than it saves (#311).

**Wheel and touch listeners are passive.** One that must call `preventDefault` is bound only while
it is needed (`zoom.svelte.ts`): a non-passive `wheel` listener on the window takes every scroll in
the app off Chromium's compositor thread.

**Full-screen elements hold still.** A moving viewport-sized layer damages the whole viewport every
frame and drags every other repaint along with it (`TheaterMode.svelte`).

**A full-view canvas draws on a budget, not once per source frame.** Every draw of the ambient
glow repaints the whole player view, in the web process and again in GTK. The AppImage runs GTK on
X11, where repainting a window that holds a GL surface costs two GLX round trips and a pixmap
readback per paint, and GTK's buffer-age damage drags a full-view repaint into the next paint or two
as well. One draw per frame of a 24 fps video cost 34% of a core on the AppImage's WebKitGTK;
capped at 15 draws a second (`GAP` in `Ambient.svelte`) it costs 24%. The glow's frame blend
smooths over 0.3 s anyway, so the extra draws bought nothing visible.

**A scroller next to animated content gets its own stacking context.** On WebKitGTK, a transform
transition blanks the scrollbar of a scroller painted after it in the same stacking context, for as
long as the transition runs. The lyrics run one per sung word, so theater mode's queue scrollbar
blinked several times a second: missing in 10 of 30 screen captures, 0 of 30 with `relative z-0` on
the queue's column (#343, `TheaterMode.svelte`). `will-change: transform` fixes it too, but costs a
layer the stacking context doesn't.

## Engine-specific CSS

`app.html` puts a `chromium` class on `<html>` under WebView2. Use it only for a difference you have
measured in both engines, and scope the WebKit-only side with `html:not(.chromium)`, as `.art-wash`
does.

## Measuring

Switch one effect off, re-measure, and compare. Every rule here came from that, and several
plausible culprits (scrollbar styling, border radius, cover images, SVG icons) measured as nothing.
Measure in the engine the report comes from, with the CPU throttled to 4x or more, and with a song
playing. Put the before/after numbers in the comment next to the effect, so the next person can see
what removing it would cost.

## Lite mode

Some machines cannot afford the effects this file prices — an iGPU starved on memory bandwidth
behind a busy dGPU, a desktop at fractional 5K scale. Lite mode is the in-app answer: one switch
(Appearance → "Lite mode"), and next to it an optional auto-arm. It keeps layout, colors, artwork
and text; it removes the constant repainting those rules above keep promising is affordable.

The toggle is **in-app state only**. The env-var alternative was measured here and lost:
`WEBKIT_DISABLE_COMPOSITING_MODE=1` at 5120x2880 is laggier than any effect, because it drops the
page to CPU rasterization wholesale. Lite mode instead removes the work, never the accelerator.

What it cuts, via a `lite` class on `<html>` plus component gates (the same split as
`prefers-reduced-motion`: CSS zeroes `animation`/`transition` and the decorative washes; JS-driven
loops are gated in code, not CSS):

- the artwork washes and their blur (player view, theater mode, playlist header) and the artwork
  accent tint — the full-window blurred layers;
- the karaoke sweep: the lyrics rAF loop is gated off (`needsFrameClock`), and the auto-scroller
  flips lines instead of gliding;
- marquee scrolls, which sit still;
- every `prefers-reduced-motion` transition too — pill fades, queue FLIP, jump-to-playing glides —
  since lite is the same "hold still" promise, taken further.

Gates read one derived value, `lite()` from `theme.svelte.ts`
(`liteMode || (liteAuto && autoArmed)`), and derive effective values only — a machine going busy
never rewrites a user's stored settings. With lite off the default path is untouched: the gates are
booleans on existing `$derived`s, no per-frame work added. Audio is out of scope by definition: the
player (`crates/player`, libmpv) is not in the diff and cannot be — lite mode is web-page only.

Auto-arm watches the DRM counters (`gpu_busy_percents`, one `read_dir` and a few four-byte reads
per 15 s poll) and hystereses them so an iGPU that spikes 0→75→0 in seconds cannot flap the mode;
constants live in one place, `lite-auto.svelte.ts`: arm after **3 consecutive samples with any
card ≥ 85 %**, disarm after **8 consecutive samples with every card < 40 %**, and never flip sides
more often than every **60 s**. The armed state is runtime-only (`autoArmed`, never persisted), so
auto-arm changes what renders, not what is saved. It exists on Linux only, where the counters do;
a machine with no readable counter — no DRM, or NVIDIA behind NVML — gets an empty reading, counts
as idle, and never arms. `LIMUSIC_FORCE_GPU_BUSY=<0-255>` overrides every reading, which is how
the hysteresis is tested without loading a GPU.
