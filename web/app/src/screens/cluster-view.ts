import type {
  ClusterIgnored,
  ClusterInfo,
  ClusterNode,
  ClusterOwn,
  ClusterSettings,
  ClusterStatus,
} from '../api/types.js'

export const CLUSTER_NODE_COUNT = 8
export const MAX_CLUSTER_NAME_BYTES = 64
export const PORT_MIN = 1
export const PORT_MAX = 65535

export type ClusterField = Exclude<keyof ClusterSettings, 'read_only' | 'block'>

export type Tone = 'plain' | 'ok' | 'warn' | 'err'

const FIELD_LABELS: [ClusterField, string][] = [
  ['enabled', 'Enabled'],
  ['cluster_name', 'Cluster name'],
  ['node_number', 'Node number'],
  ['port', 'Port'],
  ['forward_operator', 'Forward operator'],
  ['forward_home_assistant', 'Forward Home Assistant'],
  ['forward_hcl', 'Forward HCL'],
  ['forward_foreign_master', 'Forward foreign master'],
  ['send_all_lamps', 'Send “all lamps”'],
  ['accept_all_lamps', 'Accept “all lamps”'],
  ['accept_single_lamps', 'Accept single lamps'],
  ['wire_same_as', 'DALI bus'],
  ['wire_protection', 'Loop protection'],
]

export function unsavedFields(patch: Partial<ClusterSettings>): string[] {
  return FIELD_LABELS.filter(([key]) => key in patch).map(([, label]) => label)
}

export function unsavedLine(patch: Partial<ClusterSettings>): string {
  const names = unsavedFields(patch)
  if (names.length === 0) return 'No unsaved changes'
  const noun = names.length === 1 ? 'unsaved change' : 'unsaved changes'
  return `${names.length} ${noun} · ${names.join(', ')}`
}

export interface Refusal {
  field: ClusterField | null
  text: string
}

const CONFLICTS: Record<string, Refusal> = {
  cluster_name_required: {
    field: 'cluster_name',
    text: 'A cluster needs a name before it can be turned on.',
  },
  wire_names_this_node: {
    field: 'wire_same_as',
    text: 'This node cannot name its own bus: keep “own bus” or choose another node.',
  },
  wire_protection_required: {
    field: 'wire_protection',
    text: 'Loop protection stays on while the foreign master is forwarded and the bus is shared.',
  },
  cluster_node_number_pinned: {
    field: 'node_number',
    text: 'The node number is fixed while this node is the home of channels or scenes.',
  },
}

export function refusalOf(status: number | undefined, code: string, message: string): Refusal {
  if (status === 409 && code === 'conflict' && message in CONFLICTS) return CONFLICTS[message]
  if (code === 'controller_standby') {
    return { field: null, text: 'This board is the standby: change the settings on the active board.' }
  }
  if (status === 422 && code === 'invalid_value') {
    return { field: null, text: 'A value is out of range: node 0–7, port 1–65535, name up to 64 bytes.' }
  }
  const detail = message && message !== code ? ` — ${message}` : ''
  const head = status === undefined ? code : `${status} ${code}`
  return { field: null, text: `Refused: ${head}${detail}` }
}

export function forgetRefusal(node: number, status: number | undefined, message: string): string {
  if (status === 409 && message === 'node_heard') {
    return `Node ${node} is heard again; only a silent node can be forgotten.`
  }
  return `Node ${node} was not forgotten${message ? ` — ${message}` : ''}.`
}

export function forgetLabel(armed: boolean): string {
  return armed ? 'Sure?' : 'Forget'
}

export function forgettable(node: ClusterNode, readOnly: boolean): boolean {
  return !node.heard && !readOnly
}

export function utf8Bytes(text: string): number {
  return new TextEncoder().encode(text).length
}

export function clipName(raw: string): string {
  let name = raw.trim()
  while (utf8Bytes(name) > MAX_CLUSTER_NAME_BYTES) name = name.slice(0, -1)
  return name
}

export function parsePort(raw: string): number | null {
  const text = raw.trim()
  if (!/^\d+$/.test(text)) return null
  const port = Number(text)
  return port >= PORT_MIN && port <= PORT_MAX ? port : null
}

export function pinReason(info: ClusterInfo | null): string | null {
  const own = info?.nodes.find((row) => row.source === 'own')
  if (!own || own.channels + own.scenes === 0) return null
  const parts = [plural(own.channels, 'channel'), plural(own.scenes, 'scene')].filter(Boolean)
  return `Fixed while this node is the home of ${parts.join(' and ')}. Both boards of a pair share it.`
}

function plural(count: number, noun: string): string {
  if (count === 0) return ''
  return `${count} ${noun}${count === 1 ? '' : 's'}`
}

export interface Option {
  value: number | null
  label: string
}

function holderOf(status: ClusterStatus | null, node: number): string | null {
  return status?.nodes.find((row) => row.node === node)?.board ?? null
}

export function nodeNumberOptions(status: ClusterStatus | null, own: number): Option[] {
  return Array.from({ length: CLUSTER_NODE_COUNT }, (_, node) => {
    const holder = node === own ? null : holderOf(status, node)
    return { value: node, label: holder ? `${node} · held by ${holder}` : String(node) }
  })
}

export function wireOptions(status: ClusterStatus | null, own: number, current: number | null): Option[] {
  const options: Option[] = [{ value: null, label: 'Own bus' }]
  for (let node = 0; node < CLUSTER_NODE_COUNT; node++) {
    if (node === own && current !== own) continue
    const holder = holderOf(status, node)
    const suffix = node === own ? ' (this node)' : holder ? ` · ${holder}` : ''
    options.push({ value: node, label: `Same bus as node ${node}${suffix}` })
  }
  return options
}

const SECOND = 1000
const MINUTE = 60 * SECOND
const HOUR = 60 * MINUTE
const DAY = 24 * HOUR
const LONGEST_AGE_MS = 2 ** 30

export function ageText(ms: number): string {
  if (ms >= LONGEST_AGE_MS) return 'over 12 days ago'
  if (ms < MINUTE) return `${Math.floor(ms / SECOND)} s ago`
  if (ms < HOUR) return `${Math.floor(ms / MINUTE)} min ago`
  if (ms < DAY) return `${Math.floor(ms / HOUR)} h ago`
  return `${Math.floor(ms / DAY)} d ago`
}

export interface Verdict {
  tone: Tone
  text: string
}

export function heardVerdict(node: ClusterNode): Verdict {
  return { tone: node.heard ? 'ok' : 'warn', text: ageText(node.age_ms) }
}

export function nodeSummary(node: ClusterNode, own: ClusterOwn): Verdict[] {
  const out: Verdict[] = []
  if (node.split) out.push({ tone: 'err', text: 'pair split' })
  if (node.wire === node.node) out.push({ tone: 'plain', text: 'own bus' })
  else if (node.wire === own.wire) out.push({ tone: 'plain', text: 'same bus as this node' })
  else out.push({ tone: 'plain', text: `same bus as node ${node.wire}` })
  if (!node.wire_protection) out.push({ tone: 'warn', text: 'loop protection off' })
  if (node.sequence_gaps > 0) {
    out.push({ tone: 'plain', text: `${node.sequence_gaps} frame${node.sequence_gaps === 1 ? '' : 's'} missed` })
  }
  return out
}

export function showsHomeAssistant(node: ClusterNode): boolean {
  return node.home_assistant_face === true
}

export function nodeHref(node: ClusterNode): string {
  return `http://${node.address}/`
}

export function ignoredReason(row: ClusterIgnored, status: ClusterStatus): string {
  switch (row.reason) {
    case 'other_cluster':
      return 'Another cluster name: '
    case 'waiting':
      return `Waits for node ${row.node}, which another board holds.`
    case 'number_held': {
      const holder = row.node === status.own.node_number ? status.own.board : holderOf(status, row.node)
      const by = holder ? `, which ${holder} holds` : ', which another board holds'
      return `Says it is node ${row.node}${by}. Give it a free number.`
    }
  }
}

const OWN_STATE: Record<ClusterOwn['state'], (own: ClusterOwn, port: number) => Verdict> = {
  off: () => ({ tone: 'plain', text: 'off' }),
  no_address: () => ({ tone: 'warn', text: 'no network address yet' }),
  bind_failed: (_, port) => ({ tone: 'err', text: `port ${port} not bound — retrying` }),
  standby: () => ({ tone: 'plain', text: 'listening — standby' }),
  listening: () => ({ tone: 'plain', text: 'listening' }),
  waiting: (own) => ({
    tone: 'err',
    text: `waiting — node ${own.node_number} is held by ${own.number_held_by ?? 'another board'}`,
  }),
  running: () => ({ tone: 'ok', text: 'running' }),
}

export function ownVerdict(own: ClusterOwn, port: number): Verdict {
  return OWN_STATE[own.state](own, port)
}

export function executorVerdict(own: ClusterOwn): Verdict {
  if (own.state === 'off') return { tone: 'plain', text: 'cluster off' }
  if (own.wire_chain) return { tone: 'warn', text: 'this node names a bus that names another' }
  const owner = own.wire_owner
  if (!owner) return { tone: 'plain', text: 'nobody yet' }
  if (owner.this_node) return { tone: 'ok', text: 'this node executes' }
  return { tone: 'plain', text: `node ${owner.node} executes · ${owner.board}` }
}

export interface NodeLine {
  board: string | null
  text: string
  tone: Tone
  href: string
}

const CLUSTER_HREF = '#/settings/cluster'
const REDUNDANCY_HREF = '#/settings/redundancy'

export function nodeLine(status: ClusterStatus | null): NodeLine | null {
  if (!status) return null
  const own = status.own
  const board = own.board
  const n = own.node_number
  const line = (text: string, tone: Tone, href = CLUSTER_HREF): NodeLine => ({ board, text, tone, href })
  if (own.state === 'off') return null
  if (own.split_pair) return line('Pair split · see Redundancy', 'err', REDUNDANCY_HREF)
  switch (own.state) {
    case 'waiting':
      return line(`Node ${n} is taken by ${own.number_held_by ?? 'another board'}`, 'err')
    case 'bind_failed':
      return line(`Node ${n} · port not bound`, 'err')
    case 'no_address':
      return line(`Node ${n} · no network address`, 'warn')
    case 'standby':
    case 'listening':
      return line(`Node ${n} · listening`, 'plain')
    case 'running':
      return runningLine(status, line)
  }
}

function runningLine(status: ClusterStatus, line: (text: string, tone: Tone) => NodeLine): NodeLine {
  const n = status.own.node_number
  const others = status.nodes.filter((row) => row.node !== n)
  if (others.length === 0) return line(`Node ${n} · no other node heard`, 'plain')
  const heard = others.filter((row) => row.heard).length
  return line(`Node ${n} · ${heard} of ${others.length} heard`, heard < others.length ? 'warn' : 'plain')
}

export function toneClass(base: string, tone: Tone): string {
  return tone === 'plain' ? base : `${base} ${tone}`
}
