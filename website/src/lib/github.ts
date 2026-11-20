import { useEffect, useState } from 'react'

const REPO = 'SimoHypers/limusic'
export const REPO_URL = `https://github.com/${REPO}`
export const RELEASES_URL = `${REPO_URL}/releases/latest`
const ANDROID_REPO = 'SimoHypers/limusic-android'
export const ANDROID_REPO_URL = `https://github.com/${ANDROID_REPO}`
export const ANDROID_RELEASES_URL = `${ANDROID_REPO_URL}/releases/latest`

export interface RepoInfo {
  version: string | null
  stars: number | null
  deb: string | null
  rpm: string | null
  appimage: string | null
  exe: string | null
  msi: string | null
  dmg: string | null
  androidVersion: string | null
  apk: string | null
  apk32: string | null
  apkUniversal: string | null
}

const empty: RepoInfo = {
  version: null,
  stars: null,
  deb: null,
  rpm: null,
  appimage: null,
  exe: null,
  msi: null,
  dmg: null,
  androidVersion: null,
  apk: null,
  apk32: null,
  apkUniversal: null,
}

/* Fetches the latest desktop and Android releases + star count once on mount. Everything degrades to the
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
          deb: find(d.assets, n => n.endsWith('.deb')),
          rpm: find(d.assets, n => n.endsWith('.rpm')),
          appimage: find(d.assets, n => n.endsWith('.AppImage')),
          exe: find(d.assets, n => n.endsWith('.exe')),
          msi: find(d.assets, n => n.endsWith('.msi')),
          dmg: find(d.assets, n => n.endsWith('.dmg')),
        })),
      )
      .catch(() => {})

    fetch(`https://api.github.com/repos/${ANDROID_REPO}/releases/latest`)
      .then(r => (r.ok ? r.json() : Promise.reject()))
      .then(d =>
        setInfo(prev => ({
          ...prev,
          androidVersion: d.tag_name,
          apk: find(d.assets, n => n.endsWith('-arm64-v8a.apk')),
          apk32: find(d.assets, n => n.endsWith('-armeabi-v7a.apk')),
          apkUniversal: find(d.assets, n => n.endsWith('-universal.apk')),
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

export type OS = 'linux' | 'windows' | 'mac' | 'android'

export function detectOS(): OS {
  const ua = navigator.userAgent
  // Before Linux: every Android user agent also says "Linux".
  if (/Android/i.test(ua)) return 'android'
  if (/Windows/i.test(ua)) return 'windows'
  if (/Mac/i.test(ua)) return 'mac'
  return 'linux'
}
