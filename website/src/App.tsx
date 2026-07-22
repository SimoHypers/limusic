import { HugeiconsIcon } from '@hugeicons/react'
import {
  MusicNote01Icon,
  DashboardSpeed01Icon,
  AudioWave01Icon,
  QuoteDownIcon,
  UserMultiple02Icon,
  LibraryIcon,
  KeyboardIcon,
  LastFmIcon,
  Moon02Icon,
  RefreshIcon,
  PackageIcon,
  WindowsOldIcon,
  Apple01Icon,
  GithubIcon,
  PlayIcon,
  StarIcon,
  SourceCodeIcon,
} from '@hugeicons/core-free-icons'

import Aurora from '@/components/Aurora'
import SplitText from '@/components/SplitText'
import AnimatedContent from '@/components/AnimatedContent'
import FadeContent from '@/components/FadeContent'
import SpotlightCard from '@/components/SpotlightCard'
import { useGitHub, detectOS, REPO_URL, RELEASES_URL } from '@/lib/github'

import logo from '@/assets/logo.png'
import screenPlaylist from '@/assets/screen-playlist.webp'
import screenLyrics from '@/assets/screen-lyrics.webp'
import screenArtist from '@/assets/screen-artist.webp'
import screenTogether from '@/assets/screen-listen-together.webp'

const SPOTLIGHT = 'rgba(229, 72, 110, 0.16)' as const

const FEATURES = [
  {
    icon: MusicNote01Icon,
    title: 'No ads, ever',
    body: 'Limusic plays the audio stream directly, so there is nothing to interrupt. No ad breaks, no premium subscription.',
  },
  {
    icon: DashboardSpeed01Icon,
    title: 'Light on your PC',
    body: 'A native Rust core instead of a bundled browser. Opens fast, sips memory, and keeps your fans quiet.',
  },
  {
    icon: AudioWave01Icon,
    title: 'Gapless, tuned sound',
    body: 'mpv-powered audio with gapless playback, loudness normalization, and a built-in equalizer.',
  },
  {
    icon: QuoteDownIcon,
    title: 'Lyrics that follow along',
    body: 'Time-synced lyrics scroll with the song, line by line — perfect for singing along.',
  },
  {
    icon: UserMultiple02Icon,
    title: 'Listen Together',
    body: 'Host a session, share an invite code, and play music in perfect sync with friends.',
  },
  {
    icon: LibraryIcon,
    title: 'Your library, intact',
    body: 'Sign in once and your playlists, likes, albums and subscriptions are all there. Changes sync back to YouTube Music.',
  },
]

const EXTRAS = [
  { icon: KeyboardIcon, label: 'Media keys' },
  { icon: LastFmIcon, label: 'Last.fm scrobbling' },
  { icon: Moon02Icon, label: 'Themes' },
  { icon: RefreshIcon, label: 'Auto-updates' },
]

const SCREENS = [
  {
    eyebrow: 'Lyrics',
    title: 'Sing every word',
    body: 'Synced lyrics stay locked to the music. The current line lights up and the rest fades back, so you never lose your place.',
    img: screenLyrics,
    alt: 'Limusic showing time-synced lyrics',
  },
  {
    eyebrow: 'Browse',
    title: 'Go down the rabbit hole',
    body: 'Artists, albums, singles, moods and mixes — the full YouTube Music catalog in a fast native window, with search that feels instant.',
    img: screenArtist,
    alt: 'An artist page in Limusic with top songs and albums',
  },
  {
    eyebrow: 'Together',
    title: 'Press play with friends',
    body: 'Start a Listen Together session and send one invite code. Every play, skip and queue change stays in sync for everyone.',
    img: screenTogether,
    alt: 'The Listen Together dialog in Limusic',
  },
]

function Nav({ stars }: { stars: number | null }) {
  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b border-white/5 bg-background/70 backdrop-blur-md">
      <nav className="mx-auto flex h-14 max-w-6xl items-center gap-6 px-4 sm:px-6">
        <a href="#" className="flex items-center gap-2.5 font-semibold tracking-wide">
          <img src={logo} alt="" className="size-6" />
          Limusic
        </a>
        <div className="ml-auto hidden items-center gap-6 text-sm text-muted-foreground sm:flex">
          <a href="#features" className="transition-colors hover:text-foreground">Features</a>
          <a href="#screens" className="transition-colors hover:text-foreground">Screens</a>
          <a href="#download" className="transition-colors hover:text-foreground">Download</a>
        </div>
        <a
          href={REPO_URL}
          target="_blank"
          rel="noreferrer"
          className="ml-auto flex items-center gap-2 rounded-full border border-white/10 px-3.5 py-1.5 text-sm text-muted-foreground transition-colors hover:border-white/20 hover:text-foreground sm:ml-0"
        >
          <HugeiconsIcon icon={GithubIcon} size={16} strokeWidth={2} />
          <span className="hidden sm:inline">GitHub</span>
          {stars !== null && (
            <span className="flex items-center gap-1 text-xs">
              <HugeiconsIcon icon={StarIcon} size={12} strokeWidth={2} />
              {stars}
            </span>
          )}
        </a>
      </nav>
    </header>
  )
}

function Hero({ version, downloadHref, osLabel }: { version: string | null; downloadHref: string; osLabel: string }) {
  return (
    <section className="relative overflow-hidden">
      {!window.matchMedia('(prefers-reduced-motion: reduce)').matches && (
        <div className="absolute inset-0 opacity-50" aria-hidden>
          <Aurora colorStops={['#a3123f', '#ff5d8f', '#5c0a24']} amplitude={1.1} blend={0.55} speed={0.6} />
        </div>
      )}
      <div className="absolute inset-x-0 bottom-0 h-48 bg-gradient-to-b from-transparent to-background" aria-hidden />

      <div className="relative mx-auto max-w-6xl px-4 pt-36 pb-20 text-center sm:px-6 sm:pt-44">
        <FadeContent duration={800}>
          <p className="mx-auto mb-6 w-fit rounded-full border border-white/10 bg-white/5 px-4 py-1.5 text-xs tracking-widest text-muted-foreground uppercase">
            Free · Open source · Linux &amp; Windows
          </p>
        </FadeContent>

        <SplitText
          text="Your music. Ad-free. Native."
          tag="h1"
          splitType="words"
          delay={120}
          duration={1}
          className="font-heading text-4xl font-bold tracking-tight text-balance sm:text-6xl md:text-7xl"
        />

        <FadeContent duration={900} delay={400}>
          <p className="mx-auto mt-6 max-w-2xl text-base text-muted-foreground sm:text-lg">
            Limusic is a lightweight desktop player for YouTube Music. Search any song, hit play, and
            listen without ads — no browser, no premium, no bloat.
          </p>
        </FadeContent>

        <FadeContent duration={900} delay={650}>
          <div className="mt-9 flex flex-wrap items-center justify-center gap-4">
            <a
              href={downloadHref}
              className="flex items-center gap-2.5 rounded-full bg-primary-bright px-7 py-3.5 font-semibold text-primary-foreground shadow-lg shadow-primary/40 transition-transform hover:scale-105"
            >
              <HugeiconsIcon icon={PlayIcon} size={20} strokeWidth={2} fill="currentColor" />
              Download for {osLabel}
            </a>
            <a
              href={REPO_URL}
              target="_blank"
              rel="noreferrer"
              className="flex items-center gap-2.5 rounded-full border border-white/15 px-7 py-3.5 font-medium transition-colors hover:border-white/30 hover:bg-white/5"
            >
              <HugeiconsIcon icon={GithubIcon} size={20} strokeWidth={2} />
              View on GitHub
            </a>
          </div>
          <p className="mt-4 text-sm text-muted-foreground">
            {version ? `Latest release ${version}` : 'Latest release'} · macOS not packaged yet
          </p>
        </FadeContent>

        <AnimatedContent distance={80} duration={1.1} delay={0.25} scale={0.96} threshold={0}>
          <div className="mt-16">
            <img
              src={screenPlaylist}
              alt="Limusic playing a playlist, with the queue open"
              width={1920}
              height={1043}
              className="w-full rounded-xl border border-white/10 shadow-[0_0_120px_-24px_var(--primary-bright)]"
            />
          </div>
        </AnimatedContent>
      </div>
    </section>
  )
}

function Features() {
  return (
    <section id="features" className="mx-auto max-w-6xl scroll-mt-20 px-4 py-24 sm:px-6">
      <FadeContent duration={800}>
        <p className="text-center text-xs font-semibold tracking-widest text-primary-bright uppercase">Why Limusic</p>
        <h2 className="mx-auto mt-3 max-w-2xl text-center font-heading text-3xl font-bold tracking-tight text-balance sm:text-4xl">
          Everything the web player should have been
        </h2>
      </FadeContent>

      <div className="mt-14 grid gap-5 sm:grid-cols-2 lg:grid-cols-3">
        {FEATURES.map((f, i) => (
          <AnimatedContent key={f.title} distance={40} duration={0.8} delay={(i % 3) * 0.1} threshold={0.15}>
            <SpotlightCard
              spotlightColor={SPOTLIGHT}
              className="h-full !rounded-xl !border-white/10 !bg-card/60 !p-6"
            >
              <div className="mb-4 flex size-11 items-center justify-center rounded-lg bg-primary/15 text-primary-bright">
                <HugeiconsIcon icon={f.icon} size={22} strokeWidth={1.8} />
              </div>
              <h3 className="font-heading text-lg font-semibold">{f.title}</h3>
              <p className="mt-2 text-sm leading-relaxed text-muted-foreground">{f.body}</p>
            </SpotlightCard>
          </AnimatedContent>
        ))}
      </div>

      <FadeContent duration={800} delay={150}>
        <div className="mt-10 flex flex-wrap items-center justify-center gap-3">
          {EXTRAS.map(e => (
            <span
              key={e.label}
              className="flex items-center gap-2 rounded-full border border-white/10 px-4 py-2 text-sm text-muted-foreground"
            >
              <HugeiconsIcon icon={e.icon} size={16} strokeWidth={1.8} className="text-primary-bright" />
              {e.label}
            </span>
          ))}
        </div>
      </FadeContent>
    </section>
  )
}

function Screens() {
  return (
    <section id="screens" className="mx-auto max-w-6xl scroll-mt-20 px-4 py-8 sm:px-6">
      <div className="space-y-24">
        {SCREENS.map((s, i) => (
          <AnimatedContent key={s.title} distance={60} duration={0.9} threshold={0.15}>
            <div className={`flex flex-col items-center gap-8 lg:gap-14 ${i % 2 ? 'lg:flex-row-reverse' : 'lg:flex-row'}`}>
              <div className="lg:w-2/5">
                <p className="text-xs font-semibold tracking-widest text-primary-bright uppercase">{s.eyebrow}</p>
                <h3 className="mt-3 font-heading text-2xl font-bold tracking-tight sm:text-3xl">{s.title}</h3>
                <p className="mt-4 leading-relaxed text-muted-foreground">{s.body}</p>
              </div>
              <div className="lg:w-3/5">
                <img
                  src={s.img}
                  alt={s.alt}
                  width={1920}
                  height={1043}
                  loading="lazy"
                  className="w-full rounded-xl border border-white/10 shadow-2xl shadow-black/50"
                />
              </div>
            </div>
          </AnimatedContent>
        ))}
      </div>
    </section>
  )
}

interface DownloadCard {
  os: string
  icon: typeof PackageIcon
  detected: boolean
  links: { label: string; href: string | null }[]
  note?: string
}

function Download({ info, os }: { info: ReturnType<typeof useGitHub>; os: string }) {
  const cards: DownloadCard[] = [
    {
      os: 'Linux',
      icon: PackageIcon,
      detected: os === 'linux',
      links: [
        { label: '.AppImage — any distro', href: info.appimage },
        { label: '.rpm — Fedora / RHEL', href: info.rpm },
      ],
      note: 'The AppImage updates itself automatically.',
    },
    {
      os: 'Windows',
      icon: WindowsOldIcon,
      detected: os === 'windows',
      links: [
        { label: 'Installer (.exe)', href: info.exe },
        { label: 'MSI package', href: info.msi },
      ],
    },
    {
      os: 'macOS',
      icon: Apple01Icon,
      detected: os === 'mac',
      links: [{ label: 'Build from source', href: REPO_URL }],
      note: 'Not packaged yet — coming later.',
    },
  ]

  return (
    <section id="download" className="mx-auto max-w-6xl scroll-mt-20 px-4 py-24 sm:px-6">
      <FadeContent duration={800}>
        <p className="text-center text-xs font-semibold tracking-widest text-primary-bright uppercase">Download</p>
        <h2 className="mt-3 text-center font-heading text-3xl font-bold tracking-tight sm:text-4xl">Get Limusic</h2>
        <p className="mx-auto mt-4 max-w-xl text-center text-muted-foreground">
          Free and open source. Install it, sign in with your YouTube account if you want your
          library, and press play.
        </p>
      </FadeContent>

      <div className="mt-12 grid gap-5 md:grid-cols-3">
        {cards.map((c, i) => (
          <AnimatedContent key={c.os} distance={40} duration={0.8} delay={i * 0.1} threshold={0.15}>
            <div
              className={`flex h-full flex-col rounded-xl border bg-card/60 p-6 ${
                c.detected ? 'border-primary-bright/60 shadow-lg shadow-primary/20' : 'border-white/10'
              }`}
            >
              <div className="flex items-center gap-3">
                <HugeiconsIcon icon={c.icon} size={24} strokeWidth={1.8} className="text-primary-bright" />
                <h3 className="font-heading text-lg font-semibold">{c.os}</h3>
                {c.detected && (
                  <span className="ml-auto rounded-full bg-primary/20 px-2.5 py-0.5 text-xs text-primary-bright">
                    Your system
                  </span>
                )}
              </div>
              <div className="mt-5 flex flex-1 flex-col gap-2.5">
                {c.links.map(l => (
                  <a
                    key={l.label}
                    href={l.href ?? RELEASES_URL}
                    className="rounded-lg border border-white/10 px-4 py-2.5 text-center text-sm font-medium transition-colors hover:border-primary-bright/50 hover:bg-primary/10"
                  >
                    {l.label}
                  </a>
                ))}
              </div>
              {c.note && <p className="mt-4 text-xs text-muted-foreground">{c.note}</p>}
            </div>
          </AnimatedContent>
        ))}
      </div>

      <p className="mt-8 text-center text-sm text-muted-foreground">
        {info.version && <>Latest release <span className="text-foreground">{info.version}</span> · </>}
        <a href={`${REPO_URL}/releases`} target="_blank" rel="noreferrer" className="underline underline-offset-4 hover:text-foreground">
          All releases
        </a>
      </p>
    </section>
  )
}

function Footer() {
  return (
    <footer className="border-t border-white/5">
      <div className="mx-auto flex max-w-6xl flex-col items-center gap-4 px-4 py-10 text-center text-sm text-muted-foreground sm:px-6">
        <div className="flex items-center gap-2 font-semibold text-foreground">
          <img src={logo} alt="" className="size-5" />
          Limusic
        </div>
        <p className="max-w-2xl text-xs leading-relaxed">
          Limusic is an unofficial, open-source client and is not affiliated with or endorsed by
          YouTube or Google. YouTube Music is a trademark of Google LLC.
        </p>
        <div className="flex items-center gap-5">
          <a href={REPO_URL} target="_blank" rel="noreferrer" className="flex items-center gap-1.5 transition-colors hover:text-foreground">
            <HugeiconsIcon icon={GithubIcon} size={15} strokeWidth={2} /> Source
          </a>
          <a href={`${REPO_URL}/blob/master/LICENSE`} target="_blank" rel="noreferrer" className="flex items-center gap-1.5 transition-colors hover:text-foreground">
            <HugeiconsIcon icon={SourceCodeIcon} size={15} strokeWidth={2} /> GPL-3.0
          </a>
        </div>
      </div>
    </footer>
  )
}

export default function App() {
  const info = useGitHub()
  const os = detectOS()
  const osLabel = os === 'windows' ? 'Windows' : os === 'mac' ? 'macOS' : 'Linux'
  const downloadHref =
    (os === 'windows' ? info.exe : os === 'linux' ? info.appimage : null) ?? '#download'

  return (
    <>
      <Nav stars={info.stars} />
      <main>
        <Hero version={info.version} downloadHref={downloadHref} osLabel={osLabel} />
        <Features />
        <Screens />
        <Download info={info} os={os} />
      </main>
      <Footer />
    </>
  )
}
