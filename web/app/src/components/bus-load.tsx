import type { Diagnostics } from '../api/types'
import {
  absorbBusLoadSample,
  BUS_LOAD_WINDOW_MS,
  busLoadMeasured,
  createBusLoadHistory,
} from './bus-load-history'
import type { BusLoadSample } from './bus-load-history'

export { busLoadMeasured } from './bus-load-history'

const W = 720
const H = 120
const TOP = 8
const BASE = 104
const MINUTE_GRID = 5
const TICK_MIN = 10
const TICK_STEP = 4
const TICK_MAX = 20
const PERMILLE_FULL = 1000

const history = createBusLoadHistory()

export function busLoadPercent(permille: number): number {
  return Math.round(permille / 10)
}

function xOf(sample: BusLoadSample, now: number): number {
  return W * (1 - (now - sample.uptimeMs) / BUS_LOAD_WINDOW_MS)
}

function yOf(permille: number): number {
  return BASE - (Math.min(permille, PERMILLE_FULL) / PERMILLE_FULL) * (BASE - TOP)
}

function fmt(n: number): string {
  return n.toFixed(1)
}

function Grid() {
  const rows = [TOP, (TOP + BASE) / 2, BASE]
  const cols = Array.from({ length: MINUTE_GRID - 1 }, (_, i) => (W * (i + 1)) / MINUTE_GRID)
  return (
    <>
      {rows.map((y) => (
        <line class="gl" x1={0} y1={y} x2={W} y2={y} key={`r${y}`} />
      ))}
      {cols.map((x) => (
        <line class="gl" x1={x} y1={TOP} x2={x} y2={BASE} key={`c${x}`} />
      ))}
      <text class="glab" x={4} y={TOP + 10}>
        100%
      </text>
      <text class="glab" x={4} y={(TOP + BASE) / 2 + 10}>
        50%
      </text>
      <text class="glab" x={2} y={H - 4}>
        −5 min
      </text>
      <text class="glab" x={W - 26} y={H - 4}>
        now
      </text>
    </>
  )
}

function Strip({ ring }: { ring: BusLoadSample[] }) {
  const now = ring[ring.length - 1].uptimeMs
  const pts = ring.map((s) => ({ x: xOf(s, now), y: yOf(s.load), yo: yOf(s.own), s }))
  const line = pts.map((p) => `${fmt(p.x)},${fmt(p.y)}`).join(' ')
  const own = pts.map((p) => `${fmt(p.x)},${fmt(p.yo)}`).join(' ')
  const first = pts[0]
  const last = pts[pts.length - 1]
  const area =
    `M${fmt(first.x)},${BASE}` +
    pts.map((p) => ` L${fmt(p.x)},${fmt(p.y)}`).join('') +
    ` L${fmt(last.x)},${BASE} Z`
  return (
    <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label="Bus load over the last five minutes">
      <Grid />
      <path class="area" d={area} />
      <polyline class="line" points={line} />
      <polyline class="own" points={own} />
      {pts
        .filter((p) => p.s.collisions > 0)
        .map((p) => {
          const h = Math.min(TICK_MIN + TICK_STEP * (p.s.collisions - 1), TICK_MAX)
          return <line class="coll" x1={fmt(p.x)} y1={BASE} x2={fmt(p.x)} y2={BASE - h} key={p.s.uptimeMs} />
        })}
    </svg>
  )
}

export function BusLoadChart({ diag }: { diag: Diagnostics }) {
  absorbBusLoadSample(history, diag)
  const ring = history.ring
  return (
    <div class="busload">
      <div class="hd">
        <span class="t">Bus load · last 5 min</span>
        <span class="legend">
          <span>
            <i />
            load
          </span>
          <span>
            <i class="own" />
            our share
          </span>
          <span>
            <i class="coll" />
            collision
          </span>
        </span>
      </div>
      {ring.length < 2 ? (
        <div class="empty">
          {busLoadMeasured(diag)
            ? `collecting — ${ring.length} sample so far, the next arrives with the next snapshot`
            : 'not measured — the PHY has booked no tick yet'}
        </div>
      ) : (
        <Strip ring={ring} />
      )}
    </div>
  )
}
