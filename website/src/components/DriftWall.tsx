import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import './DriftWall.css'

/* React Bits' DriftWall (reactbits.dev), ported and trimmed: square tiles, themed by the site's
   tokens, hover lift in CSS, and the frame loop runs only while the wall is on screen. The wall is
   decorative (aria-hidden, tiles out of the tab order): every tile repeats a link the page already
   has. */

export type Tile = { image: string; href: string; label: string }

// Golden-ratio spread, so neighbouring columns drift at visibly different speeds.
const factor = (c: number, variance: number) => 1 + variance * ((((c * 0.618034 + 0.35) % 1) * 2) - 1)

export default function DriftWall({
  items,
  columns = 6,
  tile = 132,
  gap = 16,
  tilt = 14,
  turn = -16,
  depth = 120,
  speed = 26,
  variance = 0.45,
  parallax = 0.6,
  external = false,
}: {
  items: Tile[]
  columns?: number
  tile?: number
  gap?: number
  /* Pitch and yaw of the wall, degrees. */
  tilt?: number
  turn?: number
  /* How far the wall sits back, px. */
  depth?: number
  /* Drift in px per second; columns alternate direction around it. */
  speed?: number
  variance?: number
  /* Pointer-follow tilt strength, 0 to switch it off. */
  parallax?: number
  /* Tiles open in a new tab. */
  external?: boolean
}) {
  const root = useRef<HTMLDivElement>(null)
  const plane = useRef<HTMLDivElement>(null)
  const tracks = useRef<(HTMLDivElement | null)[]>([])
  const pointer = useRef({ x: 0, y: 0 })
  const hoveredCol = useRef(-1)
  // The lifted tile. Not CSS :hover: in the tilted wall Chromium's hit test misses the tile under
  // a resting pointer every few frames (it lands on the track or column), so the cover dropped
  // and lifted again, and its column restarted. This one changes only when the pointer is over
  // another tile, as in React Bits' version.
  const active = useRef<HTMLElement | null>(null)
  const activate = (t: HTMLElement | null) => {
    if (t === active.current) return
    active.current?.classList.remove('is-active')
    t?.classList.add('is-active')
    active.current = t
    hoveredCol.current = t ? Number(t.dataset.col) : -1
  }
  const [height, setHeight] = useState(600)

  // Each column cycles through at least six covers, so a short list doesn't stack one cover on
  // itself: with few covers every column gets the whole list, starting at a different place.
  const cols = useMemo(() => {
    const per = Math.max(Math.ceil(items.length / columns), Math.min(items.length, 6))
    return Array.from({ length: columns }, (_, c) =>
      Array.from({ length: per }, (_, j) => items[(c * per + j) % items.length]),
    )
  }, [items, columns])

  const unit = tile + gap
  const span = cols[0].length * unit
  // Enough copies of a column that, centred and shifted up by up to one copy, it still covers the
  // wall with 20% to spare for the tilt (React Bits' `1.6h / span + 1` leaves a gap at the bottom
  // when one copy is about the wall's height).
  const copies = Math.ceil((height * 1.2) / span) + 2

  useEffect(() => {
    const el = root.current!
    const ro = new ResizeObserver(([e]) => setHeight(e.contentRect.height || 600))
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  useEffect(() => {
    const offsets = cols.map((_, c) => span * ((c * 0.37) % 1))
    const vel = cols.map(() => 0)
    const tilted = { x: 0, y: 0 }
    const place = () => {
      plane.current!.style.transform =
        `translate(-50%, -50%) scale(1.18) rotateX(${tilt + tilted.y}deg) rotateY(${turn + tilted.x}deg) translateZ(${-depth}px)`
      tracks.current.forEach((t, c) => t && (t.style.transform = `translate3d(0, ${-offsets[c]}px, 0)`))
    }
    place()
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return

    let raf = 0
    let last: number | null = null
    const frame = (ts: number) => {
      const dt = last === null ? 0 : Math.min(0.05, (ts - last) / 1000)
      last = ts
      const damp = 1 - Math.exp(-dt / 0.12)
      tilted.x += (pointer.current.x * parallax * 8 - tilted.x) * damp
      tilted.y += (-pointer.current.y * parallax * 8 - tilted.y) * damp
      for (let c = 0; c < cols.length; c++) {
        // The column under the pointer eases to a stop, so its cover can be clicked.
        const target = hoveredCol.current === c ? 0 : speed * factor(c, variance) * (c % 2 ? -1 : 1)
        vel[c] += (target - vel[c]) * (1 - Math.exp(-dt / (target === 0 ? 0.16 : 0.28)))
        offsets[c] = (((offsets[c] + vel[c] * dt) % span) + span) % span
      }
      place()
      raf = requestAnimationFrame(frame)
    }
    // Only while the wall is on screen (docs/UI-PERFORMANCE.md, the rAF rule).
    const io = new IntersectionObserver(([e]) => {
      cancelAnimationFrame(raf)
      last = null
      if (e.isIntersecting) raf = requestAnimationFrame(frame)
    })
    io.observe(root.current!)
    return () => {
      io.disconnect()
      cancelAnimationFrame(raf)
    }
  }, [cols, span, tilt, turn, depth, speed, variance, parallax])

  return (
    <div
      ref={root}
      aria-hidden
      className="drift-wall"
      style={{ '--dw-tile': `${tile}px`, '--dw-gap': `${gap}px` } as CSSProperties}
      onPointerMove={e => {
        const r = e.currentTarget.getBoundingClientRect()
        pointer.current = { x: (e.clientX - r.left) / r.width - 0.5, y: (e.clientY - r.top) / r.height - 0.5 }
        const t = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>('[data-col]')
        if (t) activate(t)
      }}
      onPointerLeave={() => {
        pointer.current = { x: 0, y: 0 }
        activate(null)
      }}
    >
      <div ref={plane} className="drift-wall__plane">
        {cols.map((col, c) => (
          <div key={c} className="drift-wall__col">
            <div className="drift-wall__track" ref={el => void (tracks.current[c] = el)}>
              {Array.from({ length: copies }, (_, k) =>
                col.map((t, j) => (
                  <a
                    key={`${k}-${j}`}
                    href={t.href}
                    tabIndex={-1}
                    data-col={c}
                    draggable={false}
                    {...(external ? { target: '_blank', rel: 'noreferrer nofollow ugc' } : {})}
                    className="drift-wall__tile"
                  >
                    <span className="drift-wall__inner">
                      <img src={t.image} alt="" decoding="async" draggable={false} referrerPolicy="no-referrer" />
                      <span className="drift-wall__label">{t.label}</span>
                    </span>
                  </a>
                )),
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  )
}
