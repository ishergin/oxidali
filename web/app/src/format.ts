import type {
  Attributes,
  BankReading,
  CapabilityFlags,
  DaliWireCounters,
  ImplementedParts,
  MemoryBusUnit,
  MemoryDiagnostics,
  MemoryEnergy,
  MemoryLuminaire,
  ObservedValue,
  Operation,
  RuntimeState,
} from './api/types.js'

export const ADAPTER = 0

export const GROUP_COUNT = 16
export const SCENE_COUNT = 16
export const LEVEL_MAX = 254
export const SCENE_MASK = 255

export const hex2 = (n: number) => `0x${n.toString(16).toUpperCase().padStart(2, '0')}`
export const hex4 = (n: number) => `0x${n.toString(16).toUpperCase().padStart(4, '0')}`
export const hex6 = (n: number) => `0x${n.toString(16).toUpperCase().padStart(6, '0')}`

export const pad2 = (n: number) => String(n).padStart(2, '0')

export const hexWide = (n: number) => `0x${n.toString(16).toUpperCase()}`

export const sourceLabel = (source: string) =>
  source === 'manual_override' ? 'manual' : source

export function daliVersion(v: number | null): string {
  if (v == null || v === 0) return '—'
  if (v === 0xff) return 'unknown'
  return `${v >> 2}.${v & 3}`
}

let deviceNowMs: number | null = null
let deviceNowSampledAt = 0

export function registerDeviceNow(nowMs: number | undefined): void {
  if (typeof nowMs !== 'number' || nowMs <= 0) return
  deviceNowMs = nowMs
  deviceNowSampledAt = Date.now()
}

export function deviceNow(): number | null {
  if (deviceNowMs == null) return null
  return deviceNowMs + (Date.now() - deviceNowSampledAt)
}

export function ago(epochMs: number | null | undefined): string {
  if (epochMs == null) return 'never'
  const now = deviceNow()
  if (now == null) return '—'
  const d = Math.max(0, now - epochMs)
  if (d < 60_000) return `${Math.floor(d / 1000)} s ago`
  if (d < 3_600_000) return `${Math.floor(d / 60_000)} m ago`
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)} h ago`
  return `${Math.floor(d / 86_400_000)} d ago`
}

export function bytes(n: number): string {
  if (n < 1024) return `${n} B`
  return `${(n / 1024).toFixed(1)} KB`
}

export function uptime(ms: number): string {
  const s = Math.floor(ms / 1000)
  const d = Math.floor(s / 86400)
  const h = Math.floor((s % 86400) / 3600)
  const m = Math.floor((s % 3600) / 60)
  if (d > 0) return `${d} d ${h} h ${pad2(m)} m`
  if (h > 0) return `${h} h ${pad2(m)} m`
  return `${m} m ${pad2(s % 60)} s`
}

export function typeLabel(
  deviceType: string,
  colorMode: string,
  caps?: CapabilityFlags,
): { label: string; dt8: boolean } {
  if (deviceType === 'dt6_led') return { label: 'DT6 LED', dt8: false }
  if (deviceType === 'dt8_color') {
    const mode =
      colorMode !== 'unknown' && colorMode !== ''
        ? colorMode
        : caps?.rgb
          ? 'rgb'
          : caps?.cct
            ? 'cct'
            : caps?.xy
              ? 'xy'
              : ''
    if (mode === 'cct') return { label: 'DT8 CCT', dt8: true }
    if (mode === 'rgb') return { label: 'DT8 RGB', dt8: true }
    if (mode === 'xy') return { label: 'DT8 XY', dt8: true }
    return { label: 'DT8', dt8: true }
  }
  if (deviceType === 'unknown' || deviceType === '') return { label: 'unknown', dt8: false }
  return { label: deviceType, dt8: false }
}


const CCT_RAMP: [kelvin: number, r: number, g: number, b: number][] = [
  [2000, 255, 147, 41],
  [2700, 255, 180, 107],
  [4000, 255, 214, 170],
  [5000, 255, 241, 224],
  [6500, 202, 218, 255],
]

export function kelvinRgb(kelvin: number): [number, number, number] {
  const k = Math.min(Math.max(kelvin, CCT_RAMP[0][0]), CCT_RAMP[CCT_RAMP.length - 1][0])
  for (let i = 1; i < CCT_RAMP.length; i++) {
    const [k1, r1, g1, b1] = CCT_RAMP[i]
    if (k > k1) continue
    const [k0, r0, g0, b0] = CCT_RAMP[i - 1]
    const t = k1 === k0 ? 0 : (k - k0) / (k1 - k0)
    return [
      Math.round(r0 + (r1 - r0) * t),
      Math.round(g0 + (g1 - g0) * t),
      Math.round(b0 + (b1 - b0) * t),
    ]
  }
  const last = CCT_RAMP[CCT_RAMP.length - 1]
  return [last[1], last[2], last[3]]
}

export function rgbHex(r: number, g: number, b: number): string {
  const h = (v: number) => Math.round(Math.min(255, Math.max(0, v))).toString(16).toUpperCase().padStart(2, '0')
  return `#${h(r)}${h(g)}${h(b)}`
}

export function lampState(
  state: RuntimeState,
  verifying: boolean,
): {
  cls: string
  label: string
  colour?: string
  glow?: string
  qualifier?: string
  title: string
} {
  if (state.error?.code === 'device_absent') {
    const had = state.level != null ? ` · was ${state.level}` : ''
    return {
      cls: 'absent',
      label: 'no answer',
      qualifier: state.level != null ? `was ${state.level}` : undefined,
      title:
        `nothing answered a read addressed to this device${had}. The gear's own supply is ` +
        `the usual cause — a control gear with no mains cannot answer, which is a different ` +
        `fact from lamp failure (that one the driver reports while alive).`,
    }
  }
  if (state.power === 'off') {
    return { cls: 'off', label: 'Off', title: 'off' }
  }
  if (state.power !== 'on') {
    return { cls: 'unknown', label: 'unknown', title: 'never observed since boot' }
  }
  const observed = state.value_source === 'poller'
  const unconfirmed = !observed && !verifying
  const level = state.level
  const lit = level ?? 0
  const alpha = lit <= 0 ? 0.35 : 0.35 + 0.65 * Math.min(1, lit / LEVEL_MAX)
  let rgb: [number, number, number] = [240, 180, 41]
  let qualifier: string | undefined
  if (state.color_mode === 'cct' && state.color_temperature_kelvin != null) {
    rgb = kelvinRgb(state.color_temperature_kelvin)
    qualifier = `${state.color_temperature_kelvin} K`
  } else if ((state.color_mode === 'rgb' || state.color_mode === 'rgbwaf') && state.rgb != null) {
    const { r, g, b } = state.rgb
    const peak = Math.max(r, g, b, 1)
    rgb = [Math.round((r / peak) * 255), Math.round((g / peak) * 255), Math.round((b / peak) * 255)]
    qualifier = rgbHex(r, g, b)
  } else if (state.color_mode === 'xy' && state.xy != null) {
    qualifier = `xy ${state.xy.x.toFixed(3)} ${state.xy.y.toFixed(3)}`
  }
  const [r, g, b] = rgb
  const levelWords = level == null ? 'on, level not read' : `level ${level}`
  return {
    cls: unconfirmed ? 'on unconfirmed' : 'on',
    label: level == null ? 'on' : String(level),
    colour: `rgba(${r}, ${g}, ${b}, ${alpha.toFixed(2)})`,
    glow:
      !unconfirmed && level != null && level > LEVEL_MAX / 2
        ? `0 0 7px rgba(${r}, ${g}, ${b}, 0.45)`
        : undefined,
    qualifier,
    title: unconfirmed
      ? `commanded ${level == null ? 'on' : `to level ${level}`}${qualifier ? ` · ${qualifier}` : ''} — and nothing ` +
        `will check it: polling is off. An arc-power command carries no acknowledgment, so this ` +
        `is what was sent, not what the gear did. Enable the poller to have it read back.`
      : `on at ${levelWords}${qualifier ? ` · ${qualifier}` : ''} · ${state.value_source}`,
  }
}

export function kelvinCss(kelvin: number): string {
  const [r, g, b] = kelvinRgb(kelvin)
  return `rgb(${r}, ${g}, ${b})`
}

export function busHealth(
  wire: DaliWireCounters,
  opts: { enabled: boolean },
): { cls: string; label: string; detail: string } {
  if (!opts.enabled) {
    return {
      cls: 'idle',
      label: 'Disabled',
      detail: 'the adapter is switched off — commands are refused at the gate, nothing reaches the wire',
    }
  }
  if (wire.system_failure_active > 0) {
    return {
      cls: 'err',
      label: 'System failure',
      detail: 'the line has been held active for over 550 ms (IEC 62386-101 §4.11) — a short across the pair, or the supply is gone. No gear can be reached.',
    }
  }
  if (wire.bus_power_down_active > 0) {
    return {
      cls: 'err',
      label: 'No bus power',
      detail: 'the line has been held active for over 45 ms — bus power down. Check the DALI supply and the wiring before reading anything else on this page.',
    }
  }
  const episodes = wire.bus_power_down_entries + wire.system_failure_entries
  if (episodes > 0) {
    return {
      cls: 'warn',
      label: 'Recovered',
      detail: `the bus is up now, but it went down ${episodes} time${episodes === 1 ? '' : 's'} since boot — intermittent supply or wiring.`,
    }
  }
  const frames = wire.frames_sent_by_priority.reduce((a, b) => a + b, 0)
  if (frames === 0) {
    return {
      cls: 'idle',
      label: 'Not observed',
      detail: 'no frame has gone out since boot, so the PHY has classified nothing yet. Run a scan or set a level.',
    }
  }
  if (wire.bus_acquire_timeout > 0 || wire.retry_exhausted > 0) {
    return {
      cls: 'warn',
      label: 'Contended',
      detail: `${wire.bus_acquire_timeout} frame${wire.bus_acquire_timeout === 1 ? '' : 's'} could not take the bus and ${wire.retry_exhausted} gave up after retrying — another master, or a marginal signal.`,
    }
  }
  return {
    cls: 'ok',
    label: 'Healthy',
    detail: `${frames} frames sent, no power-down or system-failure condition seen since boot.`,
  }
}

export const STATUS_FLAG_LABELS: [key: string, label: string, cls: string, faulty: boolean][] = [
  ['lamp_failure', 'Lamp failure', 'err', true],
  ['gear_failure', 'Gear failure', 'err', true],
  ['lamp_on', 'Lamp lit', 'info', false],
  ['limit_error', 'Limit error', 'warn', true],
  ['fade_running', 'Fade running', 'info', false],
  ['reset_state', 'Reset state', 'warn', true],
  ['missing_short_address', 'No short address', 'warn', true],
  ['power_cycle_seen', 'Power cycle seen', 'warn', true],
]

export function statusSummary(state: RuntimeState): { cls: string; label: string } | null {
  if (state.error?.code === 'device_absent') return null
  const st = state.status
  if (!st) return null
  for (const [key, label, cls, faulty] of STATUS_FLAG_LABELS) {
    if (faulty && (st as unknown as Record<string, boolean>)[key]) return { cls, label }
  }
  return { cls: 'ok-txt', label: 'OK' }
}

const LIGHT_SOURCE_TYPES: Record<number, string> = {
  0: 'Fluorescent',
  2: 'HID',
  3: 'LV halogen',
  4: 'Incandescent',
  6: 'LED',
  7: 'OLED',
  252: 'Other',
  253: 'Unknown / converter',
  254: 'No light source',
}

function lightSourceName(code: number): string {
  return LIGHT_SOURCE_TYPES[code] ?? `Reserved (${code})`
}

export function lightSourceType(code: number | null, packed: number | null): string | null {
  if (code == null) return null
  if (code !== 0xff) return lightSourceName(code)
  if (packed == null) return 'Multiple'
  const slots = [(packed >> 16) & 0xff, (packed >> 8) & 0xff, packed & 0xff]
  const named = slots.filter((c, i) => !(i > 0 && (c === 254 || c === 255))).map(lightSourceName)
  return slots[2] === 255 ? `${named.join(' · ')} · …` : named.join(' · ')
}

export function attrOf(
  attributes: Attributes,
  section: string,
  field: string,
): ObservedValue<unknown> | undefined {
  return attributes[section]?.[field]
}

export function attrNum(attributes: Attributes, section: string, field: string): number | null {
  const v = attrOf(attributes, section, field)?.value
  return typeof v === 'number' ? v : null
}

export function memoryEnergyOf(attributes: Attributes): MemoryEnergy | undefined {
  return (attributes as unknown as { memory_energy?: MemoryEnergy }).memory_energy
}

export function memoryDiagnosticsOf(attributes: Attributes): MemoryDiagnostics | undefined {
  return (attributes as unknown as { memory_diagnostics?: MemoryDiagnostics }).memory_diagnostics
}

export function memoryBusUnitOf(attributes: Attributes): MemoryBusUnit | undefined {
  return (attributes as unknown as { memory_bus_unit?: MemoryBusUnit }).memory_bus_unit
}

export function memoryLuminaireOf(attributes: Attributes): MemoryLuminaire | undefined {
  return (attributes as unknown as { memory_luminaire?: MemoryLuminaire }).memory_luminaire
}

export function lightDistributionLabel(raw: number): string | null {
  const ies = ['not specified', 'Type I', 'Type II', 'Type III', 'Type IV', 'Type V', 'Type VS']
  if (raw < ies.length) return ies[raw]
  if (raw === 253) return 'emergency luminaire'
  if (raw === 254) return 'other'
  return null
}

export function implementedPartNumbers(parts: ImplementedParts): number[] {
  const bits = parts.bytes * 8
  const out: number[] = []
  for (let bit = 0; bit < bits; bit++) {
    if (parts.raw & (1 << bit)) out.push(151 + bit)
  }
  return out
}

export const TMASK_FAULT_AFTER_MS = 30_000

export const BANK_TEMPERATURE_OFFSET = 60

export type BankRender =
  | { kind: 'value'; text: string; saturated: boolean }
  | { kind: 'not_implemented' }
  | { kind: 'tmask'; stale: boolean }
  | { kind: 'unread' }

export function renderBank(
  ov: ObservedValue<BankReading> | undefined,
  now: number,
  format: (raw: number) => string,
): BankRender {
  const r = ov?.value
  if (!r) return { kind: 'unread' }
  if (r.not_implemented) return { kind: 'not_implemented' }
  if (r.temporarily_unavailable) {
    const since = r.tmask_since_ms ?? null
    return { kind: 'tmask', stale: since != null && now - since > TMASK_FAULT_AFTER_MS }
  }
  if (r.value == null) return { kind: 'unread' }
  return { kind: 'value', text: format(r.value), saturated: r.saturated }
}

export function scaleLabel(exp: number | null | undefined, unit: string): string {
  return exp == null ? unit : `x10^${exp} ${unit}`
}


export function isPreempted(
  op?: { status?: string; error?: { code?: string } } | null,
): boolean {
  return op?.status === 'failed' && op?.error?.code === 'preempted'
}

export function opStatusLabel(op: { status: string; error?: { code?: string } }): string {
  return isPreempted(op) ? 'interrupted' : op.status.replace('_', ' ')
}

export function opStatusChip(
  status: string,
  errorCode?: string,
): { cls: string; spin: boolean } {
  if (status === 'failed' && errorCode === 'preempted') return { cls: 'stood-down', spin: false }
  switch (status) {
    case 'succeeded':
      return { cls: 'ok', spin: false }
    case 'failed':
      return { cls: 'err', spin: false }
    case 'timed_out':
      return { cls: 'warn', spin: false }
    case 'running':
      return { cls: 'run', spin: true }
    case 'accepted':
      return { cls: 'idle', spin: false }
    default:
      return { cls: 'idle', spin: false }
  }
}

export function opTitle(op: Pick<Operation, 'operation_id' | 'type'>): string {
  switch (op.type) {
    case 'discovery':
      return 'Discovery'
    case 'group_apply':
      return 'Group apply'
    case 'scene_apply':
      return 'Scene apply'
    case 'attribute_read':
      return 'Attribute read'
    case 'attribute_write':
      return 'Attribute write'
    default:
      return op.type || op.operation_id
  }
}

export function opSummary(op: Operation): string | null {
  const r = op.result
  if (!r) return null
  const parts: string[] = []
  if (r.programmed_total != null) parts.push(`${r.programmed_total} programmed`)
  if (r.written_total != null) parts.push(`${r.written_total} written`)
  if (r.updated_total != null && r.updated_total > 0) parts.push(`${r.updated_total} updated`)
  if (r.cleared_total != null && r.cleared_total > 0) parts.push(`${r.cleared_total} cleared`)
  if (r.skipped_total != null && r.skipped_total > 0) parts.push(`${r.skipped_total} skipped`)
  if (r.failed_total != null && r.failed_total > 0) parts.push(`${r.failed_total} failed`)
  if (r.entities_published != null) parts.push(`${r.entities_published} announced`)
  if (r.entities_failed != null && r.entities_failed > 0) {
    parts.push(`${r.entities_failed} refused`)
  }
  return parts.length > 0 ? parts.join(' · ') : null
}

export const DEVICE_CLOCK_FLOOR_MS = 1_704_067_200_000

export function deviceClock(ms: number): { text: string; anchored: boolean } {
  if (ms >= DEVICE_CLOCK_FLOOR_MS) {
    const d = new Date(ms)
    return {
      text: `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
        + `.${String(d.getMilliseconds()).padStart(3, '0')}`,
      anchored: true,
    }
  }
  return { text: sinceBoot(ms), anchored: false }
}

function sinceBoot(ms: number): string {
  const total = Math.max(0, Math.floor(ms))
  const millis = String(total % 1000).padStart(3, '0')
  const s = Math.floor(total / 1000)
  if (s < 60) return `+${s}.${millis}`
  if (s < 3600) return `+${Math.floor(s / 60)}:${pad2(s % 60)}.${millis}`
  return `+${Math.floor(s / 3600)}:${pad2(Math.floor((s % 3600) / 60))}:${pad2(s % 60)}.${millis}`
}

export const UNANCHORED_CLOCK_HINT =
  'time since boot: the controller had not synced its clock (SNTP) when this was recorded'

export function timestamp(): string {
  const d = new Date()
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}.${String(d.getMilliseconds()).padStart(3, '0')}`
}

const RGBWAF_CONTROL_TYPE_MASK = 0xc0
const RGBWAF_CONTROL_209_CHANNEL = 0x00
const RGBWAF_CONTROL_209_COLOUR = 0x40
const RGBWAF_CONTROL_NORMALISED = 0x80
const RGBWAF_CONTROL_EXTENDED = 0xc0
const RGBWAF_CONTROL_TYPE_LABELS: Record<number, string> = {
  [RGBWAF_CONTROL_209_CHANNEL]: 'reserved (209:2011 channel control)',
  [RGBWAF_CONTROL_209_COLOUR]: 'reserved (209:2011 colour control)',
  [RGBWAF_CONTROL_NORMALISED]: 'normalised colour control',
  [RGBWAF_CONTROL_EXTENDED]: 'extended colour control',
}

export const rgbwafControlType = (b: number): string =>
  RGBWAF_CONTROL_TYPE_LABELS[b & RGBWAF_CONTROL_TYPE_MASK]

export const rgbwafDrives = (b: number): boolean => (b & 0x3f) === 0

export const rgbwafIsTarget = (b: number): boolean =>
  (b & RGBWAF_CONTROL_TYPE_MASK) === RGBWAF_CONTROL_NORMALISED && rgbwafDrives(b)

const RGBWAF_CHANNELS = ['R', 'G', 'B', 'W', 'A', 'F']

export const rgbwafChannelNames = (linked: number): string =>
  RGBWAF_CHANNELS.filter((_, i) => linked & (1 << i)).join(' ')
