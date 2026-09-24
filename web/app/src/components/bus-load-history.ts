import { deltaOf } from '../counter-delta.js'

export const BUS_LOAD_WINDOW_MS = 5 * 60_000
const RING_CAPACITY = 180
const REBOOT_GAP_MS = 90_000

export type BusLoadSample = {
  uptimeMs: number
  load: number
  own: number
  collisions: number
}

export type BusLoadHistory = {
  ring: BusLoadSample[]
  lastUptime: number | null
  lastCollisions: number | null
  rebootCandidateUptime: number | null
}

export type BusLoadDiagnostic = {
  uptime_ms: number
  dali_wire: {
    wire_ticks_total: number
    load_permille: number
    load_own_permille: number
    collisions: number
  }
}

export function createBusLoadHistory(): BusLoadHistory {
  return { ring: [], lastUptime: null, lastCollisions: null, rebootCandidateUptime: null }
}

export function busLoadMeasured(diag: Pick<BusLoadDiagnostic, 'dali_wire'>): boolean {
  return diag.dali_wire.wire_ticks_total > 0
}

function resetAfterReboot(history: BusLoadHistory): void {
  history.ring = []
  history.lastCollisions = null
  history.rebootCandidateUptime = null
}

function confirmsReboot(history: BusLoadHistory, uptime: number): boolean {
  const last = history.lastUptime
  if (last === null || uptime >= last) return false
  if (uptime + REBOOT_GAP_MS < last) return true
  const candidate = history.rebootCandidateUptime
  history.rebootCandidateUptime = uptime
  return candidate !== null && uptime > candidate
}

export function absorbBusLoadSample(history: BusLoadHistory, diag: BusLoadDiagnostic): void {
  const uptime = diag.uptime_ms
  if (confirmsReboot(history, uptime)) resetAfterReboot(history)
  else if (history.lastUptime !== null && uptime <= history.lastUptime) return
  else history.rebootCandidateUptime = null

  history.lastUptime = uptime
  if (!busLoadMeasured(diag)) return
  const total = diag.dali_wire.collisions
  const collisions =
    history.lastCollisions === null ? 0 : deltaOf(total, history.lastCollisions)
  history.lastCollisions = total
  history.ring.push({
    uptimeMs: uptime,
    load: diag.dali_wire.load_permille,
    own: diag.dali_wire.load_own_permille,
    collisions,
  })
  const horizon = uptime - BUS_LOAD_WINDOW_MS
  while (
    history.ring.length > RING_CAPACITY ||
    (history.ring.length > 0 && history.ring[0].uptimeMs < horizon)
  ) {
    history.ring.shift()
  }
}
