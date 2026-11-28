import assert from 'node:assert/strict'
import test from 'node:test'

import type { ClusterChannelState, ClusterNodePart, ClusterPartState, ClusterStatus } from '../src/api/types.js'
import {
  NOT_COMMANDED_ON,
  NO_GAPS,
  blockHolds,
  blockNote,
  channelGaps,
  channelState,
  partState,
  wireFaultText,
} from '../src/screens/cluster-state.js'

const channel = (over: Partial<ClusterChannelState>): ClusterChannelState => ({
  on: null,
  lit: 0,
  known: 0,
  members: 0,
  parts_unknown: 0,
  ...over,
})

const partSt = (over: Partial<ClusterPartState>): ClusterPartState => ({
  on: true,
  commanded: true,
  lit: 3,
  known: 4,
  members: 4,
  fresh: true,
  ...over,
})

const part = (nodeNumber: number, over: Partial<ClusterNodePart> = {}): ClusterNodePart => ({
  node: nodeNumber,
  adapter: 0,
  group: 9,
  record: { number: 24, birth_mark: { generation: 1, nonce: 5 } },
  send: true,
  accept: true,
  revision: 1,
  source: 'cached',
  link: 'live',
  wire_fault: null,
  state: partSt({}),
  ...over,
})

const heard = (nodes: [number, boolean][]): ClusterStatus => ({
  own: {
    state: 'running',
    node_number: 0,
    wire: 0,
    board: 'dali-3fa2c1',
    boot_epoch: 1,
    wire_owner: null,
    split_pair: false,
    wire_chain: false,
    unprotected_wire: false,
    number_held_by: null,
    block: 'settled',
  },
  nodes: nodes.map(([n, h]) => ({
    node: n,
    board: `dali-${n}`,
    address: `10.0.0.${n}`,
    installation: 'x',
    wire: n,
    wire_protection: true,
    home_assistant_face: null,
    heard: h,
    age_ms: 1000,
    sequence_gaps: 0,
    split: false,
  })),
  ignored: [],
})

test('a channel on with every member known counts the lit ones and says nothing more', () => {
  assert.deepEqual(channelState(channel({ on: true, lit: 7, known: 9, members: 9 }), NO_GAPS), {
    cls: 'on',
    label: '7 of 9 lit',
    tail: null,
  })
})

test('a channel off is drawn off only when the controller says off', () => {
  assert.deepEqual(channelState(channel({ on: false, lit: 0, known: 6, members: 6 }), NO_GAPS), {
    cls: 'off',
    label: '0 of 6 lit',
    tail: null,
  })
})

test('partial knowledge counts the known members and names what is missing, with the nodes not heard', () => {
  const view = channelState(channel({ on: true, lit: 3, known: 5, members: 9, parts_unknown: 1 }), { notHeard: [2], unchecked: 0 })
  assert.deepEqual(view, { cls: 'on', label: '3 of 5 known lit', tail: '4 of 9 not known · node 2 not heard' })
})

test('a part whose link cannot be checked is named beside members not known', () => {
  const view = channelState(channel({ on: null, lit: 3, known: 5, members: 9, parts_unknown: 2 }), { notHeard: [2], unchecked: 1 })
  assert.equal(view.tail, '4 of 9 not known · 1 part not known · node 2 not heard')
})

test('nothing known reads as not known, never as 0 lit or off', () => {
  const view = channelState(channel({ on: null, known: 0, members: 4, parts_unknown: 1 }), { notHeard: [2], unchecked: 0 })
  assert.deepEqual(view, { cls: 'unknown', label: 'not known', tail: '4 not known · node 2 not heard' })
  assert.notEqual(view.cls, 'off')
  assert.doesNotMatch(view.label, /\d/)
})

test('lit members without a known command leave the channel unknown, and say why', () => {
  assert.deepEqual(channelState(channel({ on: null, lit: 4, known: 4, members: 4, parts_unknown: 1 }), NO_GAPS), {
    cls: 'unknown',
    label: '4 of 4 lit',
    tail: NOT_COMMANDED_ON,
  })
})

test('a part whose link cannot be checked is counted as a part, not as members', () => {
  assert.deepEqual(channelState(channel({ on: null, parts_unknown: 1 }), { notHeard: [], unchecked: 1 }), {
    cls: 'unknown',
    label: 'not known',
    tail: '1 part not known',
  })
})

test('a channel with no members reads no lamps, and a missing state reads not known', () => {
  assert.deepEqual(channelState(channel({ on: false }), NO_GAPS), { cls: 'unknown', label: 'no lamps', tail: null })
  assert.deepEqual(channelState(undefined, NO_GAPS), { cls: 'unknown', label: 'not known', tail: null })
})

test('a stale or copied record keeps its row with the state unknown', () => {
  const lit = channel({ on: true, lit: 2, known: 2, members: 2 })
  assert.equal(channelState(lit, NO_GAPS, 'stale').cls, 'unknown')
  assert.equal(channelState(lit, NO_GAPS, 'copy').cls, 'unknown')
  assert.equal(channelState(lit, NO_GAPS, 'cached').cls, 'on')
})

test('a node not heard is named even when every counted member is known: its own members may not be counted', () => {
  const all = channel({ on: true, lit: 1, known: 1, members: 1, parts_unknown: 1 })
  assert.equal(channelState(all, { notHeard: [2], unchecked: 0 }).tail, 'node 2 not heard')
  assert.equal(channelState(all, NO_GAPS).tail, null)
})

test('a node whose part is stale, from an old set, or silent in the status is not heard', () => {
  const parts = [
    part(1),
    part(2, { source: 'stale' }),
    part(3, { state: partSt({ fresh: false }) }),
    part(4),
    part(5, { link: 'orphan', source: 'stale' }),
    part(6, { record: { number: 25, birth_mark: { generation: 1, nonce: 5 } }, source: 'stale' }),
  ]
  assert.deepEqual(channelGaps({ number: 24 }, parts, heard([[1, true], [4, false]])), { notHeard: [2, 3, 4], unchecked: 0 })
})

test('a part whose link is not checked is counted as one, and a part behind the bus owner is not counted at all', () => {
  const status = heard([[1, true], [7, false], [8, false]])
  status.nodes[2].wire = 1
  const parts = [part(1), part(7, { link: 'unknown' }), part(8, { source: 'stale' }), part(9, { link: 'unknown', record: { number: 25, birth_mark: { generation: 1, nonce: 5 } } })]
  assert.deepEqual(channelGaps({ number: 24 }, parts, status), { notHeard: [], unchecked: 1 })
})

test('a heard part lit without a command beside a silent node is not called a part not known', () => {
  const view = channelState(channel({ on: null, lit: 4, known: 4, members: 8, parts_unknown: 2 }), { notHeard: [2], unchecked: 0 })
  assert.equal(view.tail, '4 of 8 not known · node 2 not heard')
})

test("each node's part says its own state in the row's words", () => {
  assert.deepEqual(partState(part(1, { state: partSt({ on: true, lit: 4, known: 5, members: 5 }) }), false), {
    cls: 'on',
    label: '4 of 5 lit',
    tail: null,
  })
  assert.deepEqual(partState(part(1, { state: partSt({ on: null, commanded: false, lit: 4, known: 4 }) }), false), {
    cls: 'unknown',
    label: '4 of 4 lit',
    tail: 'not commanded on',
  })
  assert.deepEqual(partState(part(1, { state: partSt({ on: true, lit: 2, known: 3, members: 5 }) }), false), {
    cls: 'on',
    label: '2 of 3 known lit',
    tail: '2 of 5 not known',
  })
})

test('a silent, stale or unchecked part is not known, and an orphan shows no state', () => {
  const unknown = { cls: 'unknown', label: 'not known', tail: 'not heard' }
  assert.deepEqual(partState(part(2), true), unknown)
  assert.deepEqual(partState(part(2, { source: 'stale' }), false), unknown)
  assert.deepEqual(partState(part(2, { state: partSt({ fresh: false }) }), false), unknown)
  assert.deepEqual(partState(part(2, { link: 'unknown' }), false), { cls: 'unknown', label: 'not known', tail: 'link not checked' })
  assert.equal(partState(part(2, { link: 'orphan' }), false), null)
  const blind = partState(part(2, { state: partSt({ on: null, lit: null, known: null, members: null }) }), false)
  assert.deepEqual(blind, { cls: 'unknown', label: 'not known', tail: null })
})

test('a wire fault names the neighbour and what it links the same group to', () => {
  assert.equal(wireFaultText('channel', part(0), 'G9', 0), null)
  assert.equal(
    wireFaultText('channel', part(0, { wire_fault: { node: 2, holds: 'other_record' } }), 'G9', 0),
    'Node 2 shares this DALI bus and links G9 to another channel. The node that executes on the bus decides what G9 does.',
  )
  assert.match(wireFaultText('channel', part(0, { wire_fault: { node: 2, holds: 'nothing' } }), 'G9', 0) ?? '', /links G9 to no channel/)
  assert.match(wireFaultText('channel', part(2, { wire_fault: { node: 0, holds: 'nothing' } }), 'G9', 0) ?? '', /^This node shares/)
})

test('the create controls wait while the block is listening or contested, and say why', () => {
  assert.equal(blockHolds('settled'), false)
  assert.equal(blockHolds('listening'), true)
  assert.equal(blockHolds('contested'), true)
  assert.equal(blockNote({ state: 'running', block: 'settled' }, 0), null)
  assert.equal(blockNote(undefined, 0), null)
  assert.match(blockNote({ state: 'running', block: 'listening' }, 0) ?? '', /still listening to its neighbours/)
  assert.match(blockNote({ state: 'no_address', block: 'listening' }, 0) ?? '', /does not run in the cluster yet/)
  assert.match(blockNote({ state: 'running', block: 'contested' }, 0) ?? '', /^Another board holds node 0 as well/)
})
