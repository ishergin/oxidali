import type {
  ClusterBlockState,
  ClusterChannelState,
  ClusterNodePart,
  ClusterOwn,
  ClusterPartState,
  ClusterRecord,
  ClusterRecordKind,
  ClusterStatus,
} from '../api/types.js'
import { inListeningRound } from './cluster-view.js'

export type StateClass = 'on' | 'off' | 'unknown'

export interface StateView {
  cls: StateClass
  label: string
  tail: string | null
}

export const NOT_KNOWN = 'not known'
export const NO_LAMPS = 'no lamps'
export const NOT_COMMANDED_ON = 'not known to be on: no node knows of a command to switch it on'

const PART_NOUN = (n: number) => (n === 1 ? 'part' : 'parts')
const NODE_NOUN = (n: number) => (n === 1 ? 'node' : 'nodes')

function classOf(on: boolean | null): StateClass {
  if (on === true) return 'on'
  if (on === false) return 'off'
  return 'unknown'
}

function countLabel(lit: number, known: number, members: number): string {
  if (known === 0) return NOT_KNOWN
  return known === members ? `${lit} of ${members} lit` : `${lit} of ${known} known lit`
}

function missingMembers(known: number, members: number): string | null {
  if (members <= known) return null
  return known === 0 ? `${members} not known` : `${members - known} of ${members} not known`
}

export function notHeardText(nodes: number[]): string | null {
  if (nodes.length === 0) return null
  return `${NODE_NOUN(nodes.length)} ${nodes.join(', ')} not heard`
}

function joined(bits: (string | null)[]): string | null {
  const kept = bits.filter((b): b is string => b !== null)
  return kept.length === 0 ? null : kept.join(' · ')
}

export interface ChannelGaps {
  notHeard: number[]
  unchecked: number
}

export const NO_GAPS: ChannelGaps = { notHeard: [], unchecked: 0 }

function hasGaps(state: ClusterChannelState, gaps: ChannelGaps): boolean {
  return state.members > state.known || gaps.notHeard.length > 0 || gaps.unchecked > 0
}

function uncheckedText(unchecked: number): string | null {
  return unchecked > 0 ? `${unchecked} ${PART_NOUN(unchecked)} not known` : null
}

function channelTail(state: ClusterChannelState, gaps: ChannelGaps): string | null {
  const uncommanded = state.on === null && !hasGaps(state, gaps) && state.lit > 0 ? NOT_COMMANDED_ON : null
  return joined([
    missingMembers(state.known, state.members),
    uncheckedText(gaps.unchecked),
    notHeardText(gaps.notHeard),
    uncommanded,
  ])
}

export function channelState(
  state: ClusterChannelState | undefined,
  gaps: ChannelGaps,
  source: ClusterRecord['source'] = 'own',
): StateView {
  if (!state) return { cls: 'unknown', label: NOT_KNOWN, tail: notHeardText(gaps.notHeard) }
  if (state.members === 0 && state.parts_unknown === 0) return { cls: 'unknown', label: NO_LAMPS, tail: null }
  const trusted = source !== 'stale' && source !== 'copy'
  return {
    cls: trusted ? classOf(state.on) : 'unknown',
    label: countLabel(state.lit, state.known, state.members),
    tail: channelTail(state, gaps),
  }
}

function partCounts(st: ClusterPartState): StateView | null {
  if (st.known === null || st.members === null || st.lit === null) return null
  if (st.members === 0) return { cls: 'unknown', label: NO_LAMPS, tail: null }
  const uncommanded = st.on === null && st.commanded !== true && st.lit > 0 && st.known === st.members
  return {
    cls: classOf(st.on),
    label: countLabel(st.lit, st.known, st.members),
    tail: joined([missingMembers(st.known, st.members), uncommanded ? 'not commanded on' : null]),
  }
}

export function partState(part: ClusterNodePart, silent: boolean): StateView | null {
  if (part.link === 'orphan') return null
  if (part.link === 'unknown') return { cls: 'unknown', label: NOT_KNOWN, tail: 'link not checked' }
  const st = part.state
  if (!st || silent || part.source === 'stale' || !st.fresh) return { cls: 'unknown', label: NOT_KNOWN, tail: 'not heard' }
  return partCounts(st) ?? { cls: 'unknown', label: NOT_KNOWN, tail: null }
}

export function partSilent(part: ClusterNodePart, status: ClusterStatus | null): boolean {
  if (part.source === 'stale' || part.state?.fresh === false) return true
  const row = status?.nodes.find((n) => n.node === part.node)
  return row !== undefined && !row.heard
}

function wireOf(node: number, status: ClusterStatus | null): number | null {
  if (!status) return null
  if (status.own.node_number === node) return status.own.wire
  return status.nodes.find((n) => n.node === node)?.wire ?? null
}

export function countedParts(
  rec: Pick<ClusterRecord, 'number'>,
  parts: ClusterNodePart[],
  status: ClusterStatus | null,
): ClusterNodePart[] {
  const held = parts.filter((p) => p.record.number === rec.number && p.link !== 'orphan')
  const liveNodes = new Set(held.filter((p) => p.link === 'live').map((p) => p.node))
  const behindOwner = (node: number) => {
    const wire = wireOf(node, status)
    return wire !== null && wire !== node && liveNodes.has(wire)
  }
  return held.filter((p) => !behindOwner(p.node))
}

export function channelGaps(
  rec: Pick<ClusterRecord, 'number'>,
  parts: ClusterNodePart[],
  status: ClusterStatus | null,
): ChannelGaps {
  const counted = countedParts(rec, parts, status)
  const silent = counted.filter((p) => p.link === 'live' && partSilent(p, status)).map((p) => p.node)
  return {
    notHeard: [...new Set(silent)].sort((a, b) => a - b),
    unchecked: counted.filter((p) => p.link === 'unknown').length,
  }
}

const RECORD_NOUN: Record<ClusterRecordKind, string> = { channel: 'channel', scene: 'scene' }

export function wireFaultText(kind: ClusterRecordKind, part: ClusterNodePart, local: string, me: number): string | null {
  const fault = part.wire_fault
  if (!fault) return null
  const other = fault.holds === 'other_record' ? `another ${RECORD_NOUN[kind]}` : `no ${RECORD_NOUN[kind]}`
  const who = fault.node === me ? 'This node' : `Node ${fault.node}`
  return `${who} shares this DALI bus and links ${local} to ${other}. The node that executes on the bus decides what ${local} does.`
}

const BLOCK_LISTENING =
  'This node is still listening to its neighbours after its start and takes back the records of its block they keep a copy of. Channels and scenes of this node can be created, renamed and deleted once that round ends.'

const BLOCK_NOT_RUNNING =
  'This node does not run in the cluster yet: it has no network address, its port is not bound, or its number is taken. Channels and scenes of this node can be created, renamed and deleted once it runs; Cluster settings say why.'

export function blockNote(own: Pick<ClusterOwn, 'state' | 'block'> | undefined, node: number): string | null {
  const block = own?.block
  if (block === 'listening') return own && inListeningRound(own) ? BLOCK_LISTENING : BLOCK_NOT_RUNNING
  if (block === 'contested')
    return `Another board holds node ${node} as well: nothing of this node's block can be created, renamed or deleted until one of them gives the number up.`
  return null
}

export function blockHolds(block: ClusterBlockState | undefined): boolean {
  return block === 'listening' || block === 'contested'
}
