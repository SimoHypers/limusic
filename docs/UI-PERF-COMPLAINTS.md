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
