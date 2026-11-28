import type {
  ClusterInputDeviceInstances,
  ClusterInputInstance,
  ClusterInputsNode,
  ClusterNodeInputs,
  ClusterStatus,
} from '../api/types.js'
import { ageText } from './cluster-view.js'

export type NodeRead =
  | { kind: 'pending'; at: number }
  | { kind: 'read'; data: ClusterNodeInputs; at: number }
  | { kind: 'absent'; at: number }
  | { kind: 'failed'; at: number }

export const PENDING: NodeRead = { kind: 'pending', at: 0 }

export function newerRead(now: NodeRead | undefined, got: NodeRead): NodeRead {
  return now && now.at > got.at ? now : got
}

export interface LiveEvent {
  event: string | null
  info: number
  value: number
  at: number
}

export interface LivePayload {
  node?: number | null
  short_address?: number | null
  instance_number?: number | null
  event?: string | null
  event_info?: number | null
  value?: number | null
}

export interface InputRow {
  key: string
  address: string
  name: string | null
  haHidden: boolean
  type: string
  last: LastShown
  when: string
}

export type LastShown =
  | { kind: 'none' }
  | { kind: 'named'; text: string }
  | { kind: 'code'; code: string }

export interface NodeGroup {
  node: number
  head: string
  note: string
  stale: boolean
  rows: InputRow[]
}

const SENSOR_EVENTS: ReadonlySet<string> = new Set(['illuminance', 'position'])
const VALUE_TYPES: ReadonlySet<string> = new Set(['light_sensor', 'absolute_input'])
const CODE_DIGITS = 3
const HEX = 16

const TYPE_NAMES: Record<string, string> = {
  generic: 'generic',
  push_button: 'button',
  absolute_input: 'absolute',
  occupancy: 'occupancy',
  light_sensor: 'light',
}

export const NOTHING_ANNOUNCED = 'no input device announced: the node shares no input events, or has none'
export const NOT_READ = 'not read · the list could not be read from this node'
export const NOT_SHARED = 'not shared: other nodes do not see these devices'

export function liveKey(node: number, short: number, instance: number): string {
  return `${node}:${short}:${instance}`
}

export function liveOf(payload: LivePayload, me: number, at: number): [string, LiveEvent] | null {
  const short = payload.short_address
  const instance = payload.instance_number
  if (short === null || short === undefined || instance === null || instance === undefined) return null
  const node = payload.node ?? me
  const event = { event: payload.event ?? null, info: payload.event_info ?? 0, value: payload.value ?? 0, at }
  return [liveKey(node, short, instance), event]
}

export function inputAddress(node: number, short: number, instance: number | null): string {
  return instance === null ? `N${node}·IN${short}` : `N${node}·IN${short}/${instance}`
}

export function typeName(name: string | null): string {
  if (name === null) return 'not read'
  return TYPE_NAMES[name] ?? name.replace(/_/g, ' ')
}

function codeText(info: number): string {
  return `code 0x${info.toString(HEX).toUpperCase().padStart(CODE_DIGITS, '0')}`
}

function liveShown(live: LiveEvent): LastShown {
  if (live.event === null) return { kind: 'code', code: codeText(live.info) }
  if (SENSOR_EVENTS.has(live.event)) return { kind: 'named', text: String(live.value) }
  return { kind: 'named', text: live.event.replace(/_/g, ' ') }
}

function restShown(inst: ClusterInputInstance): LastShown {
  const rt = inst.runtime
  const valued = inst.instance_type_name !== null && VALUE_TYPES.has(inst.instance_type_name)
  if (valued && rt.input_value !== null) return { kind: 'named', text: String(rt.input_value) }
  if (rt.last_event_info === null) return { kind: 'none' }
  return { kind: 'code', code: codeText(rt.last_event_info) }
}

function liveWins(live: LiveEvent | undefined, inst: ClusterInputInstance, readAt: number): live is LiveEvent {
  if (!live) return false
  return live.at >= readAt || live.info === inst.runtime.last_event_info
}

function instanceRow(node: number, dev: ClusterInputDeviceInstances, inst: ClusterInputInstance, ctx: RowCtx): InputRow {
  const key = liveKey(node, dev.short_address, inst.instance_number)
  const live = ctx.live.get(key)
  const rt = inst.runtime
  const restAge = rt.age_ms === null ? null : rt.age_ms + Math.max(0, ctx.now - ctx.readAt)
  const fromLive = liveWins(live, inst, ctx.readAt)
  const liveAge = fromLive ? Math.max(0, ctx.now - live.at) : null
  const age = liveAge === null ? restAge : restAge === null ? liveAge : Math.min(liveAge, restAge)
  return {
    key,
    address: inputAddress(node, dev.short_address, inst.instance_number),
    name: dev.name,
    haHidden: !dev.ha_entity,
    type: typeName(inst.instance_type_name),
    last: fromLive ? liveShown(live) : restShown(inst),
    when: age === null ? '—' : ageText(age),
  }
}

function bareRow(node: number, dev: ClusterInputDeviceInstances): InputRow {
  return {
    key: `${node}:${dev.short_address}`,
    address: inputAddress(node, dev.short_address, null),
    name: dev.name,
    haHidden: !dev.ha_entity,
    type: 'not read',
    last: { kind: 'none' },
    when: '—',
  }
}

interface RowCtx {
  live: Map<string, LiveEvent>
  now: number
  readAt: number
}

function deviceRows(node: number, dev: ClusterInputDeviceInstances, ctx: RowCtx): InputRow[] {
  if (dev.instances.length === 0) return [bareRow(node, dev)]
  return dev.instances.map((inst) => instanceRow(node, dev, inst, ctx))
}

function countText(devices: number, instances: number): string {
  const d = devices === 1 ? '1 device' : `${devices} devices`
  const i = instances === 1 ? '1 instance' : `${instances} instances`
  return `${d} · ${i}`
}

export interface GroupCtx {
  me: number
  share: boolean
  status: ClusterStatus | null
  live: Map<string, LiveEvent>
  now: number
}

function nodeHead(node: number, ctx: GroupCtx): string {
  if (node === ctx.me) return `Node ${node} · this node`
  const board = ctx.status?.nodes.find((n) => n.node === node)?.board
  return board ? `Node ${node} · ${board}` : `Node ${node}`
}

function readNote(node: number, data: ClusterNodeInputs, rows: InputRow[], ctx: GroupCtx, stale: boolean): string {
  if (data.devices.length === 0 && node !== ctx.me) return NOTHING_ANNOUNCED
  const own = node === ctx.me && !ctx.share ? ` · ${NOT_SHARED}` : ''
  if (stale) return `silent ${data.age_ms === null ? '' : `${ageText(data.age_ms)} · `}devices as last heard`
  if (data.devices.length === 0) return `no input device${own}`
  return `${countText(data.devices.length, rows.length)}${own}`
}

export function nodeGroup(node: number, read: NodeRead, ctx: GroupCtx): NodeGroup {
  const head = nodeHead(node, ctx)
  if (read.kind === 'pending') return { node, head, note: 'reading…', stale: false, rows: [] }
  if (read.kind === 'absent') return { node, head, note: NOTHING_ANNOUNCED, stale: false, rows: [] }
  if (read.kind === 'failed') return { node, head, note: NOT_READ, stale: false, rows: [] }
  const stale = read.data.source === 'stale'
  const rowCtx = { live: ctx.live, now: ctx.now, readAt: read.at }
  const rows = read.data.devices.flatMap((dev) => deviceRows(node, dev, rowCtx))
  return { node, head, note: readNote(node, read.data, rows, ctx, stale), stale, rows }
}

export function inputNodes(me: number, status: ClusterStatus | null, frame: ClusterInputsNode[] | null): number[] {
  const all = [me, ...(status?.nodes ?? []).map((n) => n.node), ...(frame ?? []).map((n) => n.node)]
  return [...new Set(all)].sort((a, b) => a - b)
}

export function movedNodes(before: ClusterInputsNode[] | null, after: ClusterInputsNode[]): number[] {
  const seen = new Map((before ?? []).map((n) => [n.node, n]))
  return after
    .filter((n) => {
      const was = seen.get(n.node)
      return !was || was.set !== n.set || was.source !== n.source || was.device_count !== n.device_count
    })
    .map((n) => n.node)
}

function squash(text: string): string {
  return text.toLowerCase().replace(/[·.\s]/g, '')
}

export function rowMatches(row: InputRow, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (q === '') return true
  return (row.name ?? '').toLowerCase().includes(q) || squash(row.address).includes(squash(q))
}

export function filterGroups(groups: NodeGroup[], query: string): NodeGroup[] {
  if (query.trim() === '') return groups
  return groups
    .map((g) => ({ ...g, rows: g.rows.filter((r) => rowMatches(r, query)) }))
    .filter((g) => g.rows.length > 0)
}

export function totalsText(groups: NodeGroup[]): string {
  const instances = groups.reduce((n, g) => n + g.rows.length, 0)
  const nodes = groups.filter((g) => g.rows.length > 0).length
  const i = instances === 1 ? '1 instance' : `${instances} instances`
  return `${i} on ${nodes === 1 ? '1 node' : `${nodes} nodes`}`
}
