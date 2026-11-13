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

## Linux: presenting a frame has its own price

On Windows and macOS a frame goes from the engine to the screen on the GPU. On Linux it goes
through GTK3 first, on the UI process's main thread, and what that costs depends on the window, not
on the page. A 60px animation that costs the web process 0.4 ms a frame, WebKitGTK 2.52.1, CPU per
frame:

| path | UI process | web compositor | Xwayland |
|---|---|---|---|
| Wayland, webview with an alpha channel, 1300x720 | 3.2 ms | 0.4 ms | |
| the same at 1900x1000 | 5.4 ms | 0.4 ms | |
| Wayland, opaque webview, 1300x720 | 1.1 ms | 0.4 ms | |
| the same at 1900x1000 | 1.4 ms | 0.4 ms | |
| X11 with the NVIDIA SHM path (the AppImage), 1300x720 | 1.7 ms | 2.0 ms | 2.8 ms |
| the same at 1900x1000 | 2.4 ms | 3.4 ms | 4.4 ms |

The alpha channel is the Wayland cost: with it, GTK uploads its own drawing of the whole window as
a second texture to blend the webview over, every frame. The window is transparent so the page can
round its corners and leave a hole for mpv's picture, so the layout asks for an opaque webview
whenever neither is up (maximized, theater mode, a system frame: `set_webview_opaque`,
`routes/+layout.svelte`). A window that is not maximized still pays. Rounding it in GTK instead
(clip the toplevel's `draw` to a region of pixel rows, which keeps the texture on its direct path)
measured 1.8 ms at 1900x1000, but the edge is stepped unless GTK also paints an anti-aliased fringe
in the theme's colour; not built.

So on Linux an animation is never free, however small: anything that presents sixty frames a
second costs the UI process 7-20% of a core by itself. Prefer an effect that ends over one that
loops, and see the rule about `requestAnimationFrame` loops above.

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
