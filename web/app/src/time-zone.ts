import { pad2 } from './format.js'

export type UtcOffsetAt = (epochMs: number) => number

const SECOND_MS = 1000
const MINUTE_MS = 60 * SECOND_MS
const DAY_MS = 24 * 60 * MINUTE_MS
const MINUTES_PER_HOUR = 60
const SECONDS_PER_MINUTE = 60
const DAYS_PER_WEEK = 7
const LAST_WEEK_OF_MONTH = 5
const JANUARY = 0
const JULY = 6
const RULES_PER_YEAR = 2
const DEFAULT_DST_SHIFT_MINUTES = 60
const IN_STEP_MS = 2 * SECOND_MS
const SKEW_IN_SECONDS_BELOW_S = 120
const SKEW_IN_MINUTES_BELOW_S = 7200

interface Transition {
  atMs: number
  before: number
  after: number
}

export function browserUtcOffsetMinutes(epochMs: number): number {
  return -new Date(epochMs).getTimezoneOffset()
}

export function posixTzFromBrowser(offsetAt: UtcOffsetAt, nowMs: number): string {
  const year = new Date(nowMs).getUTCFullYear()
  const january = offsetAt(Date.UTC(year, JANUARY, 1))
  const july = offsetAt(Date.UTC(year, JULY, 1))
  if (january === july) return fixedZone(january)
  const std = Math.min(january, july)
  const dst = Math.max(january, july)
  const changes = yearTransitions(offsetAt, year)
  const start = changes.find((t) => t.before === std && t.after === dst)
  const end = changes.find((t) => t.before === dst && t.after === std)
  if (changes.length !== RULES_PER_YEAR || !start || !end) return fixedZone(offsetAt(nowMs))
  const dstOffset = dst - std === DEFAULT_DST_SHIFT_MINUTES ? '' : posixOffset(dst)
  return `${zoneName(std)}${posixOffset(std)}${zoneName(dst)}${dstOffset},${rule(start)},${rule(end)}`
}

export function civilTime(epochMs: number, offsetMinutes: number): string {
  const d = new Date(epochMs + offsetMinutes * MINUTE_MS)
  const date = `${d.getUTCFullYear()}-${pad2(d.getUTCMonth() + 1)}-${pad2(d.getUTCDate())}`
  return `${date} ${pad2(d.getUTCHours())}:${pad2(d.getUTCMinutes())}:${pad2(d.getUTCSeconds())}`
}

export function utcOffsetLabel(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? '-' : '+'
  const abs = Math.abs(offsetMinutes)
  return `UTC${sign}${pad2(Math.floor(abs / MINUTES_PER_HOUR))}:${pad2(abs % MINUTES_PER_HOUR)}`
}

export function clockSkewLabel(controllerMs: number, browserMs: number): string {
  const skew = controllerMs - browserMs
  if (Math.abs(skew) < IN_STEP_MS) return 'in step'
  const seconds = Math.round(Math.abs(skew) / SECOND_MS)
  const span =
    seconds < SKEW_IN_SECONDS_BELOW_S
      ? `${seconds} s`
      : seconds < SKEW_IN_MINUTES_BELOW_S
        ? `${Math.round(seconds / SECONDS_PER_MINUTE)} min`
        : `${Math.round(seconds / (SECONDS_PER_MINUTE * MINUTES_PER_HOUR))} h`
  return `controller ${span} ${skew > 0 ? 'ahead' : 'behind'}`
}

function fixedZone(offsetMinutes: number): string {
  return `${zoneName(offsetMinutes)}${posixOffset(offsetMinutes)}`
}

function zoneName(offsetMinutes: number): string {
  const sign = offsetMinutes < 0 ? '-' : '+'
  const abs = Math.abs(offsetMinutes)
  const minutes = abs % MINUTES_PER_HOUR
  return `<${sign}${pad2(Math.floor(abs / MINUTES_PER_HOUR))}${minutes ? pad2(minutes) : ''}>`
}

function posixOffset(offsetMinutes: number): string {
  const sign = offsetMinutes > 0 ? '-' : ''
  return `${sign}${clock(Math.abs(offsetMinutes))}`
}

function clock(minutes: number): string {
  const rest = minutes % MINUTES_PER_HOUR
  return `${Math.floor(minutes / MINUTES_PER_HOUR)}${rest ? `:${pad2(rest)}` : ''}`
}

function rule(change: Transition): string {
  const wall = new Date(change.atMs + change.before * MINUTE_MS)
  const day = wall.getUTCDate()
  const monthDays = new Date(Date.UTC(wall.getUTCFullYear(), wall.getUTCMonth() + 1, 0)).getUTCDate()
  const week = day + DAYS_PER_WEEK > monthDays ? LAST_WEEK_OF_MONTH : Math.ceil(day / DAYS_PER_WEEK)
  const minutes = wall.getUTCHours() * MINUTES_PER_HOUR + wall.getUTCMinutes()
  return `M${wall.getUTCMonth() + 1}.${week}.${wall.getUTCDay()}/${clock(minutes)}`
}

function yearTransitions(offsetAt: UtcOffsetAt, year: number): Transition[] {
  const end = Date.UTC(year + 1, JANUARY, 1)
  const changes: Transition[] = []
  let at = Date.UTC(year, JANUARY, 1)
  let offset = offsetAt(at)
  while (at < end) {
    const next = Math.min(at + DAY_MS, end)
    const nextOffset = offsetAt(next)
    if (nextOffset !== offset) changes.push(locate(offsetAt, at, next, offset))
    at = next
    offset = nextOffset
  }
  return changes
}

function locate(offsetAt: UtcOffsetAt, fromMs: number, toMs: number, before: number): Transition {
  let lo = fromMs / MINUTE_MS
  let hi = toMs / MINUTE_MS
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2)
    if (offsetAt(mid * MINUTE_MS) === before) lo = mid
    else hi = mid
  }
  return { atMs: hi * MINUTE_MS, before, after: offsetAt(hi * MINUTE_MS) }
}
