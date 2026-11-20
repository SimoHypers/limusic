import { useEffect, useState } from 'react'

/* The lyrics server's public reads (its README, "Public reads"). VITE_LYRICS_URL points a dev
   build at a local server, which needs this origin in its WEB_ORIGINS. */
const BASE = import.meta.env.VITE_LYRICS_URL ?? 'https://lyrics.limusic.app'

export type Period = 'all' | 'month'

export interface Entry {
  rank: number
  handle: string
  avatar_url: string | null
  score: number
  songs: number
  edits: number
}

export interface Work {
  video_id: string
  /* Filed by the author with the revision, unverified: shown as text, linked by video id. */
  title: string
  artist: string
  kind: 'new' | 'edit'
  at: number
}

export interface Profile {
  handle: string
  avatar_url: string | null
  joined_at: number
  score: number
  rank: number
  songs: number
  edits: number
  recent: Work[]
}

export type Load<T> = { state: 'loading' } | { state: 'ok'; data: T } | { state: 'missing' } | { state: 'error' }

/* The handle rule from PATCH /v1/me; provider-made handles fit it too. */
export const HANDLE = /^[A-Za-z0-9_]{3,20}$/

/* The songs behind the board's cover wall: the latest lyrics of each handle, taken in turn so the
   wall mixes contributors, one entry per song. The server has no feed of recent lyrics, so this
   is one profile read per handle; a profile that fails is left out.
   ponytail: a read per top contributor per visit, add a /v1/recent feed if the board gets busy. */
export async function songsOf(handles: string[], signal: AbortSignal): Promise<(Work & { handle: string })[]> {
  const lists = await Promise.all(
    handles.map(h =>
      fetch(`${BASE}/v1/users/${encodeURIComponent(h)}`, { signal })
        .then(r => (r.ok ? (r.json() as Promise<Profile>) : null))
        .catch(() => null),
    ),
  )
  const seen = new Set<string>()
  const out: (Work & { handle: string })[] = []
  for (let i = 0; lists.some(p => p && i < p.recent.length); i++)
    for (const p of lists)
      if (p && i < p.recent.length && !seen.has(p.recent[i].video_id)) {
        seen.add(p.recent[i].video_id)
        out.push({ ...p.recent[i], handle: p.handle })
      }
  return out
}

/* One GET, again whenever `attempt` changes (the Try again button). */
export function useApi<T>(path: string, attempt: number): Load<T> {
  const [load, setLoad] = useState<Load<T>>({ state: 'loading' })
  useEffect(() => {
    const ctl = new AbortController()
    setLoad({ state: 'loading' })
    fetch(BASE + path, { signal: ctl.signal })
      .then(async r => {
        if (r.status === 404) return setLoad({ state: 'missing' })
        if (!r.ok) return setLoad({ state: 'error' })
        setLoad({ state: 'ok', data: (await r.json()) as T })
      })
      .catch(() => {
        if (!ctl.signal.aborted) setLoad({ state: 'error' })
      })
    return () => ctl.abort()
  }, [path, attempt])
  return load
}
