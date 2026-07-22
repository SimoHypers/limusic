import { useEffect, useState } from 'react'

const REPO = 'SimoHypers/limusic'
export const REPO_URL = `https://github.com/${REPO}`
export const RELEASES_URL = `${REPO_URL}/releases/latest`

export interface RepoInfo {
  version: string | null
  stars: number | null
  rpm: string | null
  appimage: string | null
  exe: string | null
  msi: string | null
}

const empty: RepoInfo = { version: null, stars: null, rpm: null, appimage: null, exe: null, msi: null }

/* Fetches the latest release + star count once on mount. Everything degrades to the
   GitHub releases page if the API is unreachable or rate-limited. */
export function useGitHub(): RepoInfo {
  const [info, setInfo] = useState<RepoInfo>(empty)

  useEffect(() => {
    const find = (assets: { name: string; browser_download_url: string }[], test: (n: string) => boolean) =>
      assets.find(a => test(a.name))?.browser_download_url ?? null

    fetch(`https://api.github.com/repos/${REPO}/releases/latest`)
      .then(r => (r.ok ? r.json() : Promise.reject()))
      .then(d =>
        setInfo(prev => ({
          ...prev,
          version: d.tag_name,
          rpm: find(d.assets, n => n.endsWith('.rpm')),
          appimage: find(d.assets, n => n.endsWith('.AppImage')),
          exe: find(d.assets, n => n.endsWith('.exe')),
          msi: find(d.assets, n => n.endsWith('.msi')),
        })),
      )
      .catch(() => {})

    fetch(`https://api.github.com/repos/${REPO}`)
      .then(r => (r.ok ? r.json() : Promise.reject()))
      .then(d => setInfo(prev => ({ ...prev, stars: d.stargazers_count })))
      .catch(() => {})
  }, [])

  return info
}

export type OS = 'linux' | 'windows' | 'mac'

export function detectOS(): OS {
  const ua = navigator.userAgent
  if (/Windows/i.test(ua)) return 'windows'
  if (/Mac/i.test(ua)) return 'mac'
  return 'linux'
}
