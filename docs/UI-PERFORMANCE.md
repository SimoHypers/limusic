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

## A pass is priced per mounted element, and WebKitGTK charges the most

A change the size of one card still walks what is mounted: style from the invalidated ancestor
down, layout from the document root (nothing in the app is a layout boundary), layers for the
whole page. So what a frame costs follows how much is mounted, not how much changed, and each
engine has its own rate. Rebuilding style and layout for the same Home feed, 3795 nodes, on one
desktop (i5-8400):

| | shelves skipped by `content-visibility` | every shelf forced visible |
|---|---|---|
| Chromium 158 | 10-13 ms | 35-47 ms |
| WebKitGTK 2.52.1 | 76 ms | 219 ms |

Six times, before any effect is involved. Where that time goes on WebKitGTK, by sampled stack
(the shares overlap): resolving `::before` and `::after` styles 19%, because Tailwind's
`*, ::before, ::after` reset matches every element and WebKit then resolves both for each one;
collecting matching rules 18%; cascade and apply 23%; building renderers 12%; layout 43%, of which
updating layer positions 18% and inline SVG icons 9%. Chromium scrolls and animates off
the main thread, so the same pass shows up there as CPU, not as dropped frames, until the CPU is
weak: at 6x throttle it drops them too.

Most of the lag fixed in this file was one trigger of a page-sized pass removed at a time. The
rules below that reduce what is mounted, or stop a small event asking for a pass, are the ones
that move every surface at once.

## Rules

**Mount what the screen shows.** A shelf renders the cards its rail can show plus two; the slots
past the right edge keep their box, so the rail's width, snap points and arrows don't change, and
get their card as the rail scrolls toward them (`Shelf.svelte`). Home went from 3795 nodes to
2275, and on WebKitGTK a wheel scroll went from 62-67% of the main thread with 32-41% of frames
over 20 ms to 42% and 23%; going back to Home, worst frame 204-220 ms to 130-148 ms. On Chromium
at 6x CPU, with the `shown` rule below as well, the same scroll produced 40% more frames (p95 84
ms to 50 ms) and going back to Home went from 600-700 ms to about 400. Lists do the same with
`rows.svelte.ts`. The queue is the exception still standing: up to 200 songs it mounts every row
so an edit can slide them, and with 150 that makes opening the player view one 281-291 ms block
on WebKitGTK (98-104 ms windowed) and about 800 ms on Chromium at 6x.

**Something that appears on hover goes to `shown`, never `opacity-100`.** Write
`opacity-0 group-hover:shown`. WebKit gives an element below full opacity a layer of its own, and
the style change that creates or destroys that layer asks for a layout, which runs from the
document root: twice per card the pointer crosses, more with a play button and a menu button
fading too. `shown` is opacity 0.999 (`layout.css`), so the element never leaves its layer and the
pixels are the same. WebKitGTK, hovering along a shelf: 8.0% of the main thread to 6.4%; scrolling
under a resting pointer: 42% to 34%, frames over 20 ms 23% to 13%. Not for an element that is on
screen at rest, which would paint through an offscreen group every time.

**Nothing is inserted under the pointer.** A control that shows on hover is mounted from the start
and shown with `invisible`/`visible` or opacity. An insert dirties layout from the root, and a
shelf's arrows used to be inserted the first time each shelf was hovered or scrolled past.

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
off-screen. `Shelf`'s arrow check ran in a mount `$effect` and made going back to Home take ~870 ms
on WebKitGTK instead of ~250 ms, 640 ms of it in that one read. Measure from a `ResizeObserver` or
an `IntersectionObserver`: their callbacks run after layout, so the reads are free, and a
`ResizeObserver` also fires when `content-visibility` brings a skipped box back (`Shelf.svelte`,
one observer for every shelf). A scroll listener is the wrong place too: it runs before the
frame's layout, so a `scrollTop` read there forces one whenever anything is pending, which while
Home scrolled was a fifth of the main thread's samples. "Has this scrolled past 400px" is an
`IntersectionObserver` on a 400px strip (`routes/+page.svelte`).

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
So did, on WebKitGTK 2.52 with Home scrolling: `decoding="async"` and `loading="eager"` on covers,
`contain: strict` on the cover box, the sticky header, container queries, containing the seek bar
against its four layouts a second, and hiding the page under the open player view.
`content-visibility: auto` on shelves is worth keeping: without it the same scroll costs half
again as much.

When frames drop and no effect explains it, find out which pass is running and who asked for it.
Sample the web process's main thread (`eu-stack -1 -p <pid>` in a loop, with `DEBUGINFOD_URLS`
unset or every sample waits on a download; Fedora's WebKitGTK carries enough symbols to name
WebCore's functions) and bucket the stacks into style, layout, compositing, paint and script. A
scroll that is mostly layout is not a scroll problem. Then break on
`WebCore::RenderObject::markContainingBlocksForLayout` in `gdb` and read who dirtied it: that is
how the opacity rule and the inserted arrows were found, after three ablations had missed them.
Count CPU per thread from `/proc/<pid>/task/*/stat`, not per process: the web main thread, the
compositor thread and the UI process answer three different questions.
Measure in the engine the report comes from, with the CPU throttled to 4x or more, and with a song
playing. Put the before/after numbers in the comment next to the effect, so the next person can see
what removing it would cost.
