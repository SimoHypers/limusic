import { useEffect, useState, type CSSProperties, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  ArrowLeft01Icon,
  ArrowUpRight01Icon,
  MusicNote01Icon,
  Refresh01Icon,
  Search01Icon,
} from '@hugeicons/core-free-icons'

import logo from '@/assets/logo.png'
import { REPO_URL } from '@/lib/github'
import { HANDLE, useApi, type Entry, type Period, type Profile, type Work } from './api'

const count = (n: number, one: string, many: string) => `${n.toLocaleString('en')} ${n === 1 ? one : many}`
const ordinal = (n: number) => {
  const suffix = { one: 'st', two: 'nd', few: 'rd', other: 'th' } as const
  return `${n}${suffix[new Intl.PluralRules('en', { type: 'ordinal' }).select(n) as keyof typeof suffix]}`
}
const month = (unix: number) =>
  new Date(unix * 1000).toLocaleDateString('en', { month: 'long', year: 'numeric' })
const day = (unix: number) => new Date(unix * 1000).toLocaleDateString('en', { month: 'short', day: 'numeric' })

/* A headline word. `glue` sits it against the word before (punctuation), `href` makes it a link. */
type Word = { text: string; href?: string; glue?: boolean }

const words = (s: string): Word[] => s.split(' ').map(text => ({ text }))
/* For an empty board, or a server that didn't answer. */
const SLOGAN = 'Synced by the people who listen.'

/* "Synced by a, b, and c.", the credit line under the lyrics in the app, with the board's top
   three as the names. Each name links to its profile. */
function credit(entries: Entry[]): Word[] {
  const out = words('Synced by')
  const names = entries.slice(0, 3).map(e => e.handle)
  for (const p of new Intl.ListFormat('en', { type: 'conjunction' }).formatToParts(names)) {
    if (p.type === 'element') out.push({ text: p.value, href: `?u=${encodeURIComponent(p.value)}` })
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
  const style = { '--t': `${Math.max(0, at - (inserted - born))}ms`, '--d': `${d}ms` } as CSSProperties
  const span = (
    <span className="sweep-word" style={style}>
      {word.text}
    </span>
  )
  return (
    <>
      {word.glue ? null : ' '}
      {word.href ? (
        <a
          href={word.href}
          className="decoration-primary-bright decoration-[0.06em] underline-offset-[0.12em] hover:underline focus-visible:outline-2 focus-visible:outline-primary-bright"
        >
          {span}
        </a>
      ) : (
        span
      )}
    </>
  )
}

function Sweep({ line }: { line: Word[] }) {
  const [born] = useState(() => performance.now())
  let t = 300
  return (
    <h1 className="min-h-[2.1em] font-heading text-[clamp(2.25rem,8.5vw,4.5rem)] leading-[1.05] font-bold tracking-tight text-balance [overflow-wrap:anywhere]">
      {line.map((w, i) => {
        // Roughly how long the word takes to say; punctuation is a blink.
        const d = w.glue ? 60 : 140 + w.text.length * 45
        const at = t
        t += d + (w.glue ? 0 : 40)
        return <SweepWord key={i} word={w} at={at} d={d} born={born} />
      })}
    </h1>
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
      <nav className="mx-auto flex h-14 max-w-3xl items-center gap-3 px-4 sm:px-6">
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
          className="ml-auto rounded-full border border-border px-3.5 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
        >
          Get Limusic
        </a>
      </nav>
    </header>
  )
}

function Footer() {
  return (
    <footer className="mt-28 border-t border-border">
      <div className="mx-auto flex max-w-3xl flex-wrap items-center justify-between gap-4 px-4 py-8 text-sm text-muted-foreground sm:px-6">
        <p>Limusic is free and open source. Its lyrics are synced by the people who use it.</p>
        <a href={REPO_URL} target="_blank" rel="noreferrer" className="transition-colors hover:text-foreground">
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

function Board() {
  const [period, setPeriod] = useState<Period>('all')
  const [attempt, setAttempt] = useState(0)
  const load = useApi<{ entries: Entry[] }>(`/v1/leaderboard?period=${period}&limit=100`, attempt)
  // The headline credits the all-time top three, so it is fixed by the first all-time answer and
  // doesn't follow the period switch.
  const [line, setLine] = useState<Word[]>(() => words('Synced by'))
  const [lineSet, setLineSet] = useState(false)
  if (!lineSet && (period !== 'all' || load.state !== 'loading')) {
    setLineSet(true)
    const top = period === 'all' && load.state === 'ok' ? load.data.entries : []
    setLine(top.length ? credit(top) : words(SLOGAN))
  }

  return (
    <>
      <section className="mx-auto max-w-3xl px-4 pt-16 pb-12 sm:px-6 sm:pt-24">
        <Sweep line={line} />
        <p className="mt-6 max-w-xl leading-relaxed text-muted-foreground">
          The word-by-word lyrics in Limusic are written and timed by its listeners. Every lyric that
          gets approved, and every vote it earns, counts here.
        </p>
      </section>

      <section className="mx-auto max-w-3xl px-4 sm:px-6" aria-label="Leaderboard">
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

        {/* Column names, unless there are no rows to name. */}
        <div
          hidden={load.state === 'error' || load.state === 'missing' || (load.state === 'ok' && !load.data.entries.length)}
          className="mt-8 grid grid-cols-[2.25rem_1fr_auto] items-end gap-x-3 px-2 pb-2 text-xs text-muted-foreground sm:grid-cols-[2.25rem_1fr_4.5rem_4.5rem_5rem]">
          <span className="text-right">Rank</span>
          <span className="pl-[52px]">Contributor</span>
          <span className="hidden text-right sm:block">Songs</span>
          <span className="hidden text-right sm:block">Edits</span>
          <span className="text-right">Points</span>
        </div>

        {load.state === 'error' || load.state === 'missing' ? (
          <Retry onRetry={() => setAttempt(a => a + 1)} />
        ) : load.state === 'loading' ? (
          <ol aria-busy className="border-t border-border">
            {Array.from({ length: 8 }, (_, i) => (
              <li key={i} className="flex h-16 items-center gap-3 border-b border-border px-2">
                <span className="ml-auto h-3 w-5 rounded bg-muted" />
                <span className="size-10 rounded-full bg-muted" />
                <span className="h-3.5 w-32 rounded bg-muted" />
                <span className="ml-auto h-3.5 w-12 rounded bg-muted" />
              </li>
            ))}
          </ol>
        ) : load.data.entries.length === 0 ? (
          <p className="border-t border-border py-16 text-center text-muted-foreground">
            {period === 'month'
              ? 'No points yet this month. The first lyric approved this month puts its author on top.'
              : 'Nobody is on the board yet.'}
          </p>
        ) : (
          <ol className="border-t border-border">
            {load.data.entries.map(e => (
              <li key={e.handle} className="border-b border-border">
                <a
                  href={`?u=${encodeURIComponent(e.handle)}`}
                  className="grid min-h-16 grid-cols-[2.25rem_1fr_auto] items-center gap-x-3 rounded-lg px-2 py-2.5 hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-primary-bright sm:grid-cols-[2.25rem_1fr_4.5rem_4.5rem_5rem]"
                >
                  <span
                    className={`text-right font-heading tabular-nums ${
                      e.rank <= 3 ? 'text-lg font-bold text-primary-bright' : 'text-muted-foreground'
                    }`}
                  >
                    {e.rank}
                  </span>
                  <span className="flex min-w-0 items-center gap-3">
                    <Avatar url={e.avatar_url} handle={e.handle} className="size-10 text-base" px={40} />
                    <span className="min-w-0">
                      <span className="block truncate font-medium">{e.handle}</span>
                      <span className="block text-xs text-muted-foreground sm:hidden">
                        {count(e.songs, 'song', 'songs')}, {count(e.edits, 'edit', 'edits')}
                      </span>
                    </span>
                  </span>
                  <span className="hidden text-right text-muted-foreground tabular-nums sm:block">
                    {e.songs.toLocaleString('en')}
                  </span>
                  <span className="hidden text-right text-muted-foreground tabular-nums sm:block">
                    {e.edits.toLocaleString('en')}
                  </span>
                  <span className="text-right font-heading font-semibold tabular-nums">{e.score.toLocaleString('en')}</span>
                </a>
              </li>
            ))}
          </ol>
        )}
      </section>

      <section className="mx-auto mt-24 max-w-3xl px-4 sm:px-6">
        <h2 className="font-heading text-2xl font-bold tracking-tight">Get on the board</h2>
        <ol className="mt-6 grid gap-6 sm:grid-cols-3">
          {[
            'Play a song in Limusic for desktop and open its lyrics.',
            'Click the lyrics’ source under them, choose Sync these lyrics, and tap along with each word.',
            'Link Discord or GitHub in Settings, Contributing. Until you do, your lyrics still count, just without your name.',
          ].map((step, i) => (
            <li key={i} className="border-t-2 border-primary-bright/70 pt-4">
              <span className="font-heading text-sm font-semibold text-primary-bright">{i + 1}</span>
              <p className="mt-1.5 text-sm leading-relaxed text-muted-foreground">{step}</p>
            </li>
          ))}
        </ol>
        <p className="mt-10 max-w-2xl text-sm leading-relaxed text-muted-foreground">
          A song nobody had synced earns the most points, an edit to an existing lyric fewer, and votes from
          other listeners add or take away. A lyric that is rejected or taken down costs points.
        </p>
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

/* YouTube's thumbnail for the video id, as the app's review page does (`coverOf`): free for us,
   about 10 KB a cover for the visitor, only for rows on screen, and no referrer. 16:9, so the
   middle square is cropped, which is the whole cover for a song. Taken from the id rather than
   the filed title, so a wrong title still sits beside the real song's cover. */
function Cover({ id }: { id: string }) {
  const [broken, setBroken] = useState(false)
  return (
    <span className="flex size-12 shrink-0 items-center justify-center overflow-hidden rounded-md bg-muted text-muted-foreground">
      {broken || !/^[\w-]{11}$/.test(id) ? (
        <HugeiconsIcon icon={MusicNote01Icon} size={20} strokeWidth={1.8} />
      ) : (
        <img
          src={`https://i.ytimg.com/vi/${id}/mqdefault.jpg`}
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

  useEffect(() => {
    document.title = found ? `${found.handle} on Limusic Lyrics` : 'Limusic Lyrics'
  }, [found])

  return (
    <section className="mx-auto max-w-3xl px-4 pt-10 sm:px-6 sm:pt-14">
      <a href="./" className="inline-flex items-center gap-1.5 text-sm text-muted-foreground transition-colors hover:text-foreground">
        <HugeiconsIcon icon={ArrowLeft01Icon} size={16} strokeWidth={2} />
        Leaderboard
      </a>

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
      ) : !found ? (
        <div aria-busy className="mt-10 flex items-center gap-5">
          <span className="size-22 rounded-full bg-muted" />
          <span className="h-8 w-48 rounded bg-muted" />
        </div>
      ) : (
        <>
          <header className="mt-10 flex items-center gap-4 sm:gap-5">
            <Avatar url={found.avatar_url} handle={found.handle} className="size-16 text-2xl sm:size-22 sm:text-4xl" px={88} />
            <div className="min-w-0">
              <h1 className="font-heading text-3xl font-bold tracking-tight [overflow-wrap:anywhere] sm:text-5xl">{found.handle}</h1>
              <p className="mt-1 text-muted-foreground">Joined {month(found.joined_at)}</p>
            </div>
          </header>
          <Summary p={found} />
          <Recent items={found.recent} />
        </>
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
