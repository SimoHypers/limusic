import { useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  ArrowLeft01Icon,
  ArrowUpRight01Icon,
  Calendar03Icon,
  ChampionIcon,
  GithubIcon,
  MusicNote01Icon,
  Refresh01Icon,
  Search01Icon,
  TouchInteraction01Icon,
  UserAdd01Icon,
} from '@hugeicons/core-free-icons'

import logo from '@/assets/logo.png'
import DriftWall, { type Tile } from '@/components/DriftWall'
import { REPO_URL } from '@/lib/github'
import { HANDLE, songsOf, useApi, type Entry, type Period, type Profile, type Work } from './api'

const count = (n: number, one: string, many: string) => `${n.toLocaleString('en')} ${n === 1 ? one : many}`
const ordinal = (n: number) => {
  const suffix = { one: 'st', two: 'nd', few: 'rd', other: 'th' } as const
  return `${n}${suffix[new Intl.PluralRules('en', { type: 'ordinal' }).select(n) as keyof typeof suffix]}`
}
const month = (unix: number) =>
  new Date(unix * 1000).toLocaleDateString('en', { month: 'long', year: 'numeric' })
const day = (unix: number) => new Date(unix * 1000).toLocaleDateString('en', { month: 'short', day: 'numeric' })

/* A headline word. `glue` sits it against the word before (punctuation), `href` makes it a link. */
type Word = { text: string; href?: string; glue?: boolean; tone?: string }

const words = (s: string): Word[] => s.split(' ').map(text => ({ text }))
/* For an empty board, or a server that didn't answer. */
const SLOGAN = 'Synced by the people who listen.'

/* First, second and third in the headline: the accent, then orchid and amber beside it. */
const TONES = ['var(--primary-bright)', 'var(--orchid)', 'var(--amber)']

/* "Synced by a, b, and c.", the credit line under the lyrics in the app, with the board's top
   three as the names. Each name links to its profile and sings in its own colour. */
function credit(entries: Entry[]): Word[] {
  const out = words('Synced by')
  const names = entries.slice(0, 3).map(e => e.handle)
  let n = 0
  for (const p of new Intl.ListFormat('en', { type: 'conjunction' }).formatToParts(names)) {
    if (p.type === 'element') out.push({ text: p.value, href: `?u=${encodeURIComponent(p.value)}`, tone: TONES[n++] })
    else for (const tok of p.value.split(/(,)|\s+/).filter(Boolean)) out.push({ text: tok, glue: tok === ',' })
  }
  out.push({ text: '.', glue: true })
  return out
}

/* One word of the headline, filling left to right like a sung word in the app's lyrics view.
   `at` is when it should start, counted from when the headline first drew. A word that arrives
   later (the names, once the board answers) takes off the time already gone, so the line keeps
   its pace instead of pausing for the network. */
function SweepWord({ word, at, d, born }: { word: Word; at: number; d: number; born: number }) {
  const [inserted] = useState(() => performance.now())
  const style = {
    '--t': `${Math.max(0, at - (inserted - born))}ms`,
    '--d': `${d}ms`,
    ...(word.tone && { '--sung': word.tone }),
  } as CSSProperties
  const span = (
    <span className={word.href ? 'sweep-word is-name' : 'sweep-word'} style={style}>
      {word.text}
    </span>
  )
  return (
    <>
      {word.glue ? null : ' '}
      {word.href ? (
        <a
          href={word.href}
          style={{ textDecorationColor: word.tone }}
          className="decoration-[0.06em] underline-offset-[0.12em] hover:underline focus-visible:outline-2 focus-visible:outline-primary-bright"
        >
          {span}
        </a>
      ) : (
        span
      )}
    </>
  )
}

/* When each word of a line starts and how long it takes: roughly how long it takes to say,
   punctuation a blink. Returns the times from the line's start, and the line's length. */
function timing(line: Word[]) {
  let t = 0
  const out = line.map(w => {
    const d = w.glue ? 60 : 140 + w.text.length * 45
    const at = t
    t += d + (w.glue ? 0 : 40)
    return { at, d }
  })
  return { words: out, length: t }
}

/* A line of SweepWords, starting `start` ms in. Inside a `.sweep-wait` it holds until the class
   goes. */
function SweepLine({ line, start = 0 }: { line: Word[]; start?: number }) {
  const [born] = useState(() => performance.now())
  return (
    <>
      {timing(line).words.map(({ at, d }, i) => (
        <SweepWord key={i} word={line[i]} at={300 + start + at} d={d} born={born} />
      ))}
    </>
  )
}

/* YouTube's thumbnail for a video id, as the app's review page does (`coverOf`): free for us,
   about 10 KB a cover for the visitor, and sent with no referrer. 16:9, so the middle square is
   cropped, which is the whole cover for a song. Taken from the id rather than the filed title,
   so a wrong title still sits beside the real song's cover. */
const coverOf = (id: string) => (/^[\w-]{11}$/.test(id) ? `https://i.ytimg.com/vi/${id}/mqdefault.jpg` : null)

/* Fewer covers than this and the wall would mostly repeat itself, so the hero goes without. */
const MIN_WALL = 4

/* The top of both pages: the text on the left, a wall of covers drifting off the right edge (under
   the text on a phone). `tiles` null is still loading: the room for the wall is kept so the text
   doesn't jump when it comes. */
function Hero({ tiles, external, children }: { tiles: Tile[] | null; external?: boolean; children: ReactNode }) {
  const split = tiles === null || tiles.length >= MIN_WALL
  return (
    <section className="relative">
      <div
        className={`pointer-events-none relative z-10 mx-auto max-w-5xl px-4 sm:px-6 ${
          split ? 'pt-12 sm:pt-16 lg:flex lg:min-h-[600px] lg:items-center lg:pt-0' : 'pt-16 sm:pt-24'
        }`}
      >
        <div className={`pointer-events-auto ${split ? 'lg:w-[48%]' : 'max-w-3xl'}`}>{children}</div>
      </div>
      {split && (
        <div className="mt-6 h-72 sm:h-96 lg:absolute lg:inset-y-0 lg:right-0 lg:mt-0 lg:h-auto lg:w-[56%]">
          {tiles && <DriftWall items={tiles} external={external} />}
        </div>
      )}
    </section>
  )
}

/* `className` sets the size and the fallback letter's size, `px` the image's intrinsic size. */
function Avatar({ url, handle, className, px }: { url: string | null; handle: string; className: string; px: number }) {
  const [broken, setBroken] = useState(false)
  if (!url || broken)
    return (
      <span
        aria-hidden
        className={`flex shrink-0 items-center justify-center rounded-full bg-muted font-heading font-semibold text-muted-foreground uppercase ${className}`}
      >
        {handle[0]}
      </span>
    )
  return (
    <img
      src={url}
      alt=""
      width={px}
      height={px}
      loading="lazy"
      onError={() => setBroken(true)}
      className={`shrink-0 rounded-full bg-muted object-cover ${className}`}
    />
  )
}

function Retry({ onRetry }: { onRetry: () => void }) {
  return (
    <div className="py-16 text-center">
      <p className="text-muted-foreground">The lyrics server didn’t answer. It may be busy, try again in a minute.</p>
      <button
        type="button"
        onClick={onRetry}
        className="mt-5 inline-flex items-center gap-2 rounded-full border border-border px-5 py-2.5 text-sm font-medium transition-colors hover:bg-muted"
      >
        <HugeiconsIcon icon={Refresh01Icon} size={16} strokeWidth={2} />
        Try again
      </button>
    </div>
  )
}

function Header() {
  return (
    <header className="border-b border-border">
      <nav className="mx-auto flex h-14 max-w-5xl items-center gap-3 px-4 sm:px-6">
        <a href="../" className="flex items-center gap-2.5 font-semibold tracking-wide">
          <img src={logo} alt="" className="size-6" />
          Limusic
        </a>
        <span aria-hidden className="text-muted-foreground/60">/</span>
        <a href="./" className="text-muted-foreground transition-colors hover:text-foreground">
          Lyrics
        </a>
        <a
          href="../#download"
          className="ml-auto inline-flex items-center gap-1.5 rounded-full border border-border px-3.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          Get Limusic
          <HugeiconsIcon icon={ArrowUpRight01Icon} size={15} strokeWidth={2} />
        </a>
      </nav>
    </header>
  )
}

function Footer() {
  return (
    <footer className="mt-28 border-t border-border">
      <div className="mx-auto flex max-w-5xl flex-wrap items-center justify-between gap-4 px-4 py-8 text-sm text-muted-foreground sm:px-6">
        <p>Limusic is free and open source. Its lyrics are synced by the people who use it.</p>
        <a
          href={REPO_URL}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1.5 transition-colors hover:text-foreground"
        >
          <HugeiconsIcon icon={GithubIcon} size={15} strokeWidth={2} />
          Source on GitHub
        </a>
      </div>
    </footer>
  )
}

const PERIODS: { id: Period; label: string }[] = [
  { id: 'all', label: 'All time' },
  { id: 'month', label: 'This month' },
]

/* "8 songs, 2 edits", leaving out a count that is zero. */
const work = (e: { songs: number; edits: number }) =>
  [e.songs && count(e.songs, 'song', 'songs'), e.edits && count(e.edits, 'edit', 'edits')].filter(Boolean).join(', ')

/* "80 points" with the figure lit, like the profile's summary. */
function Points({ n }: { n: number }) {
  return (
    <span className="text-sm whitespace-nowrap text-muted-foreground">
      <span className="font-heading text-base font-semibold text-foreground tabular-nums">{n.toLocaleString('en')}</span>{' '}
      {n === 1 ? 'point' : 'points'}
    </span>
  )
}

/* Step heights for first, second and third. */
const STEP = ['h-28 sm:h-32', 'h-20 sm:h-24', 'h-14 sm:h-16']

/* The top three on steps, first in the middle with the trophy (every rank 1, if it's a tie). A
   place nobody holds yet stays open and points at how to take it. */
function Podium({ entries }: { entries: Entry[] }) {
  return (
    <ol className="mx-auto mt-12 grid max-w-2xl grid-cols-3 items-end gap-2 sm:gap-4">
      {[0, 1, 2].map(i => {
        const e = entries[i]
        const first = e?.rank === 1
        return (
          <li key={i} className={`flex min-w-0 flex-col items-center text-center ${['order-2', 'order-1', 'order-3'][i]}`}>
            {e ? (
              <a
                href={`?u=${encodeURIComponent(e.handle)}`}
                className="group flex w-full min-w-0 flex-col items-center px-1 pb-4 focus-visible:outline-2 focus-visible:outline-primary-bright"
              >
                {first && <HugeiconsIcon icon={ChampionIcon} size={30} strokeWidth={1.8} className="mb-3 text-gold" />}
                <Avatar
                  url={e.avatar_url}
                  handle={e.handle}
                  px={96}
                  className={
                    first
                      ? 'size-18 text-3xl ring-2 ring-gold ring-offset-4 ring-offset-background sm:size-24'
                      : 'size-14 text-xl sm:size-18 sm:text-2xl'
                  }
                />
                <span className="mt-4 w-full truncate font-medium group-hover:underline">{e.handle}</span>
                <Points n={e.score} />
              </a>
            ) : (
              <a href="#sync" className="group flex flex-col items-center px-1 pb-4 text-sm text-muted-foreground">
                <span className="flex size-14 items-center justify-center rounded-full border-2 border-dashed border-border group-hover:text-foreground sm:size-18">
                  <HugeiconsIcon icon={UserAdd01Icon} size={22} strokeWidth={1.8} />
                </span>
                <span className="mt-4 group-hover:text-foreground">Still open</span>
              </a>
            )}
            <span
              aria-hidden
              className={`flex w-full justify-center rounded-t-xl bg-muted/70 pt-3 font-heading text-2xl font-bold text-muted-foreground tabular-nums sm:text-3xl ${STEP[i]}`}
            >
              {e?.rank ?? i + 1}
            </span>
          </li>
        )
      })}
    </ol>
  )
}

/* Fourth place down. */
function Row({ e }: { e: Entry }) {
  return (
    <li className="border-b border-border">
      <a
        href={`?u=${encodeURIComponent(e.handle)}`}
        className="grid min-h-16 grid-cols-[2.5rem_1fr_auto] items-center gap-x-3 rounded-lg px-2 py-2.5 hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-primary-bright sm:grid-cols-[2.5rem_1fr_11rem_7rem]"
      >
        <span className="text-center font-heading text-muted-foreground tabular-nums">{e.rank}</span>
        <span className="flex min-w-0 items-center gap-3">
          <Avatar url={e.avatar_url} handle={e.handle} className="size-10 text-base" px={40} />
          <span className="min-w-0">
            <span className="block truncate font-medium">{e.handle}</span>
            <span className="block text-xs text-muted-foreground sm:hidden">{work(e)}</span>
          </span>
        </span>
        <span className="hidden text-sm text-muted-foreground sm:block">{work(e)}</span>
        <span className="text-right">
          <Points n={e.score} />
        </span>
      </a>
    </li>
  )
}

const STEPS = [
  'Play a song in Limusic for desktop and open its lyrics.',
  'Click the source under the lyrics, choose Sync these lyrics, and tap along with each word.',
  'Link Discord or GitHub in Settings, Contributing, and your name goes up on this board.',
]

/* The steps as lyric lines, sung one after another like a song, once, when they scroll into view. */
function SungLines({ lines }: { lines: string[] }) {
  const ref = useRef<HTMLOListElement>(null)
  const [go, setGo] = useState(false)
  useEffect(() => {
    const io = new IntersectionObserver(
      ([e]) => {
        if (!e.isIntersecting) return
        setGo(true)
        io.disconnect()
      },
      { rootMargin: '0px 0px -15% 0px', threshold: 0.4 },
    )
    io.observe(ref.current!)
    return () => io.disconnect()
  }, [])
  let start = 0
  return (
    <ol
      ref={ref}
      className={`mx-auto mt-6 max-w-2xl space-y-3 font-heading text-[clamp(1.125rem,2.2vw,1.5rem)] leading-snug font-semibold tracking-tight text-balance ${
        go ? '' : 'sweep-wait'
      }`}
    >
      {lines.map(l => {
        const line = words(l)
        const at = start
        start += timing(line).length + 300
        return (
          <li key={l}>
            <SweepLine line={line} start={at} />
          </li>
        )
      })}
    </ol>
  )
}

/* An invented line for the visitor to sync: one tap per word. */
const DEMO = 'Every word lands right where you tap'.split(' ')
const stamp = (ms: number) => `${Math.floor(ms / 60000)}:${((ms / 1000) % 60).toFixed(2).padStart(5, '0')}`

function TapAlong() {
  // When each word was tapped, from performance.now().
  const [taps, setTaps] = useState<number[]>([])
  const done = taps.length === DEMO.length
  return (
    <div className="mx-auto mt-10 max-w-2xl rounded-2xl bg-background/60 p-5 sm:p-7">
      <p className="text-sm text-muted-foreground" aria-live="polite">
        {done ? 'That’s syncing. Do it to a real song in Limusic and it counts.' : 'Try it here: tap once for each word, in time.'}
      </p>
      <p className="mt-4 flex flex-wrap justify-center gap-x-2.5 gap-y-2 font-heading text-xl font-semibold tracking-tight sm:text-2xl">
        {DEMO.map((w, i) => (
          <span key={i} className="flex flex-col items-center">
            <span
              className={
                i < taps.length
                  ? 'tap-word'
                  : i === taps.length
                    ? 'text-muted-foreground underline decoration-primary-bright decoration-2 underline-offset-8'
                    : 'text-muted-foreground/50'
              }
            >
              {w}
            </span>
            <span className="mt-1.5 h-4 text-xs font-medium text-primary-bright tabular-nums">
              {i < taps.length ? stamp(taps[i] - taps[0]) : ''}
            </span>
          </span>
        ))}
      </p>
      <button
        type="button"
        onClick={() => {
          const now = performance.now()
          // Finished, or back after more than a pause between words: this tap starts the line over.
          setTaps(t => (t.length === DEMO.length ? [] : t.length && now - t[t.length - 1] > 5000 ? [now] : [...t, now]))
        }}
        className="mt-5 inline-flex items-center gap-2 rounded-full bg-primary-bright px-6 py-2.5 font-semibold text-primary-foreground shadow-lg shadow-primary/30 transition-transform active:scale-95 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary-bright"
      >
        <HugeiconsIcon icon={TouchInteraction01Icon} altIcon={Refresh01Icon} showAlt={done} size={18} strokeWidth={2} />
        {done ? 'Start over' : taps.length ? 'Tap' : 'Tap to start'}
      </button>
    </div>
  )
}

/* The panel's backdrop: a few of the board's covers blurred into a wash of their colours, like the
   app's player view, over a crimson glow that stands in when there are none. Static, and blurred
   from 320 px thumbnails, so it is one raster (docs/UI-PERFORMANCE.md, blur a small source once). */
const WASH = [
  { left: '-15%', top: '-35%' },
  { right: '-20%', top: '-10%' },
  { left: '25%', bottom: '-45%' },
]

function Wash({ covers }: { covers: string[] }) {
  return (
    <div aria-hidden className="absolute inset-0 -z-10">
      <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_20%_0%,color-mix(in_oklch,var(--primary-bright)_40%,transparent),transparent_65%)]" />
      {covers.slice(0, 3).map((src, i) => (
        <img
          key={src}
          src={src}
          alt=""
          referrerPolicy="no-referrer"
          style={WASH[i]}
          className="absolute h-[90%] w-[65%] object-cover opacity-75 blur-[70px] saturate-150"
        />
      ))}
      <div className="absolute inset-0 bg-background/50" />
    </div>
  )
}

/* The server's scoring defaults (POINTS_* and VOTE_POINTS_* in its README); keep in step if they
   change on the VPS. */
const SCORING: { n: string; what: string; gain: boolean }[] = [
  { n: '+10', what: 'for a song nobody had synced', gain: true },
  { n: '+3', what: 'for an edit to someone else’s lyric', gain: true },
  { n: '+20', what: 'at most from listeners’ votes on one lyric, and −10 at worst', gain: true },
  { n: '−2', what: 'for a lyric that is rejected', gain: false },
  { n: '−5', what: 'for one taken down, which also loses what it earned', gain: false },
]

function Scoring() {
  return (
    <div className="mt-10">
      <ul className="grid grid-cols-2 gap-x-6 gap-y-6 text-center sm:grid-cols-3 lg:grid-cols-5">
        {SCORING.map(s => (
          <li key={s.n}>
            <span className={`font-heading text-3xl font-bold tabular-nums ${s.gain ? 'text-gold' : 'text-muted-foreground'}`}>
              {s.n}
            </span>
            <p className="mt-1 text-sm leading-snug text-muted-foreground">{s.what}</p>
          </li>
        ))}
      </ul>
      <p className="mx-auto mt-8 max-w-2xl text-center text-sm leading-relaxed text-muted-foreground">
        Until you link an account your lyrics still count, just without your name on them.
      </p>
    </div>
  )
}

function Board() {
  const [period, setPeriod] = useState<Period>('all')
  const [attempt, setAttempt] = useState(0)
  const load = useApi<{ entries: Entry[] }>(`/v1/leaderboard?period=${period}&limit=100`, attempt)
  // The headline credits the all-time top three and the wall shows the songs of the all-time top
  // five, each cover crediting who synced it. Both are fixed by the first good all-time answer, so
  // they don't follow the period switch, and a failed first answer doesn't stop a retry filling
  // them. Until then the headline waits on "Synced by" while the all-time board loads, and says the
  // slogan otherwise (server down, or "This month" picked first), as it does for an empty board.
  const [top, setTop] = useState<Entry[] | null>(null)
  if (!top && period === 'all' && load.state === 'ok') setTop(load.data.entries)
  const waiting = !top && period === 'all' && load.state === 'loading'
  const line = useMemo(
    () => (top?.length ? credit(top) : words(waiting ? 'Synced by' : SLOGAN)),
    [top, waiting],
  )
  const [tiles, setTiles] = useState<Tile[] | null>(null)
  useEffect(() => {
    if (!top) return
    const ctl = new AbortController()
    songsOf(top.slice(0, 5).map(e => e.handle), ctl.signal).then(songs => {
      if (ctl.signal.aborted) return
      const out: Tile[] = []
      for (const s of songs) {
        const image = coverOf(s.video_id)
        if (image) out.push({ image, href: `?u=${encodeURIComponent(s.handle)}`, label: `Synced by ${s.handle}` })
      }
      setTiles(out.slice(0, 40))
    })
    return () => ctl.abort()
  }, [top])

  return (
    <>
      <Hero tiles={top ? tiles : waiting ? null : []}>
        <h1 className="min-h-[2.1em] font-heading text-[clamp(2.25rem,5.2vw,3.5rem)] leading-[1.05] font-bold tracking-tight text-balance [overflow-wrap:anywhere]">
          <SweepLine line={line} />
        </h1>
        <p className="mt-6 max-w-md leading-relaxed text-muted-foreground">
          The word-by-word lyrics in Limusic are written and timed by its listeners. Every lyric that
          gets approved, and every vote it earns, counts here.
        </p>
        <a
          href="#sync"
          className="mt-5 inline-block text-sm font-medium text-primary-bright underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-primary-bright"
        >
          How to get on the board
        </a>
      </Hero>

      <section className="mx-auto mt-10 max-w-5xl px-4 sm:px-6 lg:mt-4" aria-label="Leaderboard">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex rounded-full border border-border p-1 text-sm" role="group" aria-label="Period">
            {PERIODS.map(p => (
              <button
                key={p.id}
                type="button"
                aria-pressed={period === p.id}
                onClick={() => setPeriod(p.id)}
                className={`rounded-full px-4 py-1.5 transition-colors ${
                  period === p.id ? 'bg-muted font-medium text-foreground' : 'text-muted-foreground hover:text-foreground'
                }`}
              >
                {p.label}
              </button>
            ))}
          </div>
          <form method="get" action="./" role="search" className="relative w-full sm:w-auto">
            <HugeiconsIcon
              icon={Search01Icon}
              size={16}
              strokeWidth={2}
              className="pointer-events-none absolute top-1/2 left-3.5 -translate-y-1/2 text-muted-foreground"
            />
            <input
              name="u"
              required
              pattern="[A-Za-z0-9_]{3,20}"
              title="3 to 20 letters, digits or underscores"
              placeholder="Find a contributor"
              aria-label="Find a contributor by name"
              autoComplete="off"
              spellCheck={false}
              className="h-10 w-full rounded-full sm:w-56 border border-border bg-transparent pr-4 pl-10 text-sm outline-none placeholder:text-muted-foreground focus-visible:border-primary-bright"
            />
          </form>
        </div>

        {load.state === 'error' || load.state === 'missing' ? (
          <Retry onRetry={() => setAttempt(a => a + 1)} />
        ) : load.state === 'loading' ? (
          <ol aria-busy className="mt-10 border-t border-border">
            {Array.from({ length: 6 }, (_, i) => (
              <li key={i} className="flex h-16 items-center gap-3 border-b border-border px-2">
                <span className="mx-2.5 h-3 w-5 rounded bg-muted" />
                <span className="size-10 rounded-full bg-muted" />
                <span className="h-3.5 w-32 rounded bg-muted" />
                <span className="ml-auto h-3.5 w-16 rounded bg-muted" />
              </li>
            ))}
          </ol>
        ) : load.data.entries.length === 0 ? (
          <p className="mt-10 border-t border-border py-16 text-center text-muted-foreground">
            {period === 'month'
              ? 'No points yet this month. The first lyric approved this month puts its author on top.'
              : 'Nobody is on the board yet.'}
          </p>
        ) : (
          <>
            <Podium entries={load.data.entries} />
            {load.data.entries.length > 3 && (
              <ol className="mt-10 border-t border-border">
                {load.data.entries.slice(3).map(e => (
                  <Row key={e.handle} e={e} />
                ))}
              </ol>
            )}
          </>
        )}
      </section>

      <section id="sync" className="mx-auto mt-28 max-w-5xl scroll-mt-8 px-4 sm:px-6">
        <div className="relative isolate overflow-hidden rounded-3xl border border-border px-5 py-10 text-center sm:px-12 sm:py-14">
          <Wash covers={(tiles ?? []).map(t => t.image)} />
          <h2 className="font-heading text-xl font-bold tracking-tight sm:text-2xl">Get on the board</h2>
          <SungLines lines={STEPS} />
          <TapAlong />
        </div>
        <Scoring />
      </section>
    </>
  )
}

/* "Synced 38 songs and made 12 edits." with the figures lit and the words around them dim,
   like the sung and unsung parts of a lyric line. */
function Summary({ p }: { p: Profile }) {
  const lit = (s: string) => <span className="whitespace-nowrap text-foreground">{s}</span>
  const work: ReactNode =
    p.songs && p.edits ? (
      <>Synced {lit(count(p.songs, 'song', 'songs'))} and made {lit(count(p.edits, 'edit', 'edits'))}.</>
    ) : p.songs ? (
      <>Synced {lit(count(p.songs, 'song', 'songs'))}.</>
    ) : p.edits ? (
      <>Made {lit(count(p.edits, 'edit', 'edits'))} to synced lyrics.</>
    ) : (
      <>No lyrics live yet.</>
    )
  return (
    <p className="mt-10 max-w-2xl font-heading text-2xl leading-snug font-semibold tracking-tight text-balance text-muted-foreground sm:text-3xl">
      <span className="block">{work}</span>
      <span className="block">
        {p.score > 0 ? (
          <>
            {lit(count(p.score, 'point', 'points'))}, {lit(ordinal(p.rank))} on the board.
          </>
        ) : (
          <>No points yet.</>
        )}
      </span>
    </p>
  )
}

/* A song's cover beside its row, or a note when there is none. */
function Cover({ id }: { id: string }) {
  const [broken, setBroken] = useState(false)
  const src = coverOf(id)
  return (
    <span className="flex size-12 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted text-muted-foreground">
      {broken || !src ? (
        <HugeiconsIcon icon={MusicNote01Icon} size={20} strokeWidth={1.8} />
      ) : (
        <img
          src={src}
          alt=""
          width={48}
          height={48}
          loading="lazy"
          decoding="async"
          referrerPolicy="no-referrer"
          onError={() => setBroken(true)}
          className="size-full object-cover"
        />
      )}
    </span>
  )
}

function Recent({ items }: { items: Work[] }) {
  if (items.length === 0) return null
  const groups: { label: string; items: Work[] }[] = []
  for (const w of items) {
    const label = month(w.at)
    if (groups.at(-1)?.label !== label) groups.push({ label, items: [] })
    groups.at(-1)!.items.push(w)
  }
  return (
    <section className="mt-16">
      <h2 className="font-heading text-xl font-bold tracking-tight">Recent lyrics</h2>
      {groups.map(g => (
        <div key={g.label} className="mt-8">
          <h3 className="mb-2 text-sm text-muted-foreground">{g.label}</h3>
          <ul className="border-t border-border">
            {g.items.map((w, i) => (
              <li key={`${w.video_id}-${w.at}-${i}`} className="border-b border-border">
                {/* The title is what the author filed with the lyric. The link goes by video id,
                    so the song it opens is the real one whatever the title says. */}
                <a
                  href={`https://music.youtube.com/watch?v=${encodeURIComponent(w.video_id)}`}
                  target="_blank"
                  rel="noreferrer nofollow ugc"
                  className="group grid grid-cols-[auto_1fr_auto] items-center gap-x-4 rounded-lg px-2 py-2.5 hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-primary-bright"
                >
                  <Cover id={w.video_id} />
                  <span className="min-w-0">
                    <span className="block truncate font-medium" title={w.title}>
                      {w.title}
                    </span>
                    <span className="block truncate text-sm text-muted-foreground" title={w.artist}>
                      {w.artist}
                    </span>
                  </span>
                  <span className="flex items-center gap-3 text-right text-sm text-muted-foreground">
                    <span>
                      <span className="block">{w.kind === 'new' ? 'Synced' : 'Edited'}</span>
                      <span className="block text-xs">{day(w.at)}</span>
                    </span>
                    <HugeiconsIcon
                      icon={ArrowUpRight01Icon}
                      size={16}
                      strokeWidth={2}
                      className="opacity-50 group-hover:opacity-100"
                      aria-label="Opens on YouTube Music"
                    />
                  </span>
                </a>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </section>
  )
}

function ProfilePage({ handle }: { handle: string }) {
  const valid = HANDLE.test(handle)
  const [attempt, setAttempt] = useState(0)
  const load = useApi<Profile>(`/v1/users/${encodeURIComponent(valid ? handle : '_')}`, attempt)
  const found = valid && load.state === 'ok' ? load.data : null
  // Their songs for the wall, each cover opening the song like the rows below.
  const tiles = useMemo(() => {
    const out: Tile[] = []
    const seen = new Set<string>()
    for (const w of found?.recent ?? []) {
      const image = coverOf(w.video_id)
      if (!image || seen.has(w.video_id)) continue
      seen.add(w.video_id)
      out.push({ image, href: `https://music.youtube.com/watch?v=${encodeURIComponent(w.video_id)}`, label: w.title })
    }
    return out
  }, [found])

  useEffect(() => {
    document.title = found ? `${found.handle} on Limusic Lyrics` : 'Limusic Lyrics'
  }, [found])

  const back = (
    <a href="./" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground">
      <HugeiconsIcon icon={ArrowLeft01Icon} size={16} strokeWidth={2} />
      Leaderboard
    </a>
  )

  if (found)
    return (
      <>
        <div className="mx-auto max-w-5xl px-4 pt-10 sm:px-6">{back}</div>
        <Hero tiles={tiles} external>
          <header className="flex items-center gap-4 sm:gap-5">
            <Avatar url={found.avatar_url} handle={found.handle} className="size-16 text-2xl sm:size-22 sm:text-4xl" px={88} />
            <div className="min-w-0">
              <h1 className="font-heading text-3xl font-bold tracking-tight [overflow-wrap:anywhere] sm:text-5xl">{found.handle}</h1>
              <p className="mt-1 inline-flex items-center gap-1.5 text-muted-foreground">
                <HugeiconsIcon icon={Calendar03Icon} size={16} strokeWidth={1.8} />
                Joined {month(found.joined_at)}
              </p>
            </div>
          </header>
          <Summary p={found} />
        </Hero>
        <div className="mx-auto max-w-5xl px-4 sm:px-6">
          <div className="max-w-3xl">
            <Recent items={found.recent} />
          </div>
        </div>
      </>
    )

  return (
    <section className="mx-auto max-w-5xl px-4 pt-10 sm:px-6 sm:pt-14">
      {back}
      {!valid || load.state === 'missing' ? (
        <div className="pt-16">
          <h1 className="font-heading text-4xl font-bold tracking-tight">No one by that name</h1>
          <p className="mt-4 max-w-xl leading-relaxed text-muted-foreground">
            A contributor shows up here once they link Discord or GitHub in Limusic. Check the spelling, or find
            them on the <a href="./" className="text-foreground underline underline-offset-4">leaderboard</a>.
          </p>
        </div>
      ) : load.state === 'error' ? (
        <Retry onRetry={() => setAttempt(a => a + 1)} />
      ) : (
        <div aria-busy className="mt-10 flex items-center gap-5">
          <span className="size-22 rounded-full bg-muted" />
          <span className="h-8 w-48 rounded bg-muted" />
        </div>
      )}
    </section>
  )
}

export default function Lyrics() {
  const handle = new URLSearchParams(location.search).get('u')?.trim()
  return (
    <div className="flex min-h-dvh flex-col">
      <Header />
      <main className="flex-1">{handle ? <ProfilePage handle={handle} /> : <Board />}</main>
      <Footer />
    </div>
  )
}
