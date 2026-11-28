import assert from 'node:assert/strict'
import test from 'node:test'

import type { ClusterInputDeviceInstances, ClusterInputInstance, ClusterNodeInputs } from '../src/api/types.js'
import {
  type GroupCtx,
  type LiveEvent,
  NOTHING_ANNOUNCED,
  NOT_READ,
  NOT_SHARED,
  filterGroups,
  inputNodes,
  liveOf,
  movedNodes,
  newerRead,
  nodeGroup,
  totalsText,
  typeName,
} from '../src/screens/cluster-inputs-view.js'

const READ_AT = 100_000

const instance = (n: number, over: Partial<ClusterInputInstance> = {}): ClusterInputInstance => ({
  instance_number: n,
  instance_type: 1,
  instance_type_name: 'push_button',
  instance_groups: [null],
  runtime: { last_event_info: 2, input_value: null, age_ms: 12_000 },
  ...over,
})

const device = (short: number, instances: ClusterInputInstance[], over: Partial<ClusterInputDeviceInstances> = {}) => ({
  node: 0,
  source: 'own' as const,
  adapter_id: 0,
  short_address: short,
  name: 'Hall panel',
  present: true,
  instance_count: instances.length,
  ha_entity: true,
  instances,
  ...over,
})

const inputs = (nodeNumber: number, devices: ClusterInputDeviceInstances[], over: Partial<ClusterNodeInputs> = {}) => ({
  node: nodeNumber,
  source: nodeNumber === 0 ? ('own' as const) : ('cached' as const),
  set: 3,
  age_ms: 2000,
  device_count: devices.length,
  owner: null,
  devices,
  ...over,
})

const ctx = (over: Partial<GroupCtx> = {}): GroupCtx => ({
  me: 0,
  share: true,
  status: null,
  live: new Map<string, LiveEvent>(),
  now: READ_AT,
  ...over,
})

test("every instance of a node's devices is a row with its cluster address, type and last event", () => {
  const data = inputs(0, [device(3, [instance(0), instance(1, { runtime: { last_event_info: null, input_value: null, age_ms: null } })])])
  const group = nodeGroup(0, { kind: 'read', data, at: READ_AT }, ctx())
  assert.equal(group.head, 'Node 0 · this node')
  assert.equal(group.note, '1 device · 2 instances')
  assert.deepEqual(
    group.rows.map((r) => [r.address, r.type, r.last, r.when]),
    [
      ['N0·IN3/0', 'button', { kind: 'code', code: 'code 0x002' }, '12 s ago'],
      ['N0·IN3/1', 'button', { kind: 'none' }, '—'],
    ],
  )
})

test('the REST age counts on from the moment it was read', () => {
  const data = inputs(0, [device(3, [instance(0)])])
  const group = nodeGroup(0, { kind: 'read', data, at: READ_AT }, ctx({ now: READ_AT + 50_000 }))
  assert.equal(group.rows[0].when, '1 min ago')
})

test('a live event names the press and wins over a list read before it', () => {
  const data = inputs(1, [device(5, [instance(0)], { node: 1, name: 'Storage panel' })])
  const entry = liveOf({ node: 1, short_address: 5, instance_number: 0, event: 'short_press', event_info: 2, value: 0 }, 0, READ_AT + 1000)
  assert.ok(entry)
  const live = new Map([entry])
  const group = nodeGroup(1, { kind: 'read', data, at: READ_AT }, ctx({ live, now: READ_AT + 3000 }))
  assert.deepEqual([group.rows[0].last, group.rows[0].when], [{ kind: 'named', text: 'short press' }, '2 s ago'])
})

test('a live event older than the list and of another code gives way to the list', () => {
  const data = inputs(1, [device(5, [instance(0, { runtime: { last_event_info: 12, input_value: null, age_ms: 0 } })], { node: 1 })])
  const entry = liveOf({ node: 1, short_address: 5, instance_number: 0, event: 'short_press', event_info: 2, value: 0 }, 0, READ_AT - 5000)
  assert.ok(entry)
  const group = nodeGroup(1, { kind: 'read', data, at: READ_AT }, ctx({ live: new Map([entry]) }))
  assert.deepEqual(group.rows[0].last, { kind: 'code', code: 'code 0x00C' })
})

test("this node's own event carries no node and lands on this node's row; one without an instance lands nowhere", () => {
  assert.equal(liveOf({ short_address: 3, instance_number: 1, event: 'press', event_info: 1 }, 0, 1)?.[0], '0:0:3:1')
  assert.equal(liveOf({ short_address: 3, instance_number: null, event: 'press', event_info: 1 }, 0, 1), null)
})

test('a sensor shows its value, live and from the list', () => {
  const sensor = instance(0, { instance_type_name: 'light_sensor', runtime: { last_event_info: 0, input_value: 412, age_ms: 40_000 } })
  const data = inputs(1, [device(6, [sensor], { node: 1 })])
  const group = nodeGroup(1, { kind: 'read', data, at: READ_AT }, ctx())
  assert.deepEqual([group.rows[0].type, group.rows[0].last], ['light', { kind: 'named', text: '412' }])
  const entry = liveOf({ node: 1, short_address: 6, instance_number: 0, event: 'illuminance', event_info: 0, value: 380 }, 0, READ_AT + 1)
  assert.ok(entry)
  const live = nodeGroup(1, { kind: 'read', data, at: READ_AT }, ctx({ live: new Map([entry]) }))
  assert.deepEqual(live.rows[0].last, { kind: 'named', text: '380' })
})

test('a silent node keeps its devices, marked as last heard', () => {
  const data = inputs(2, [device(1, [instance(0)], { node: 2, source: 'stale' })], { source: 'stale', age_ms: 240_000 })
  const group = nodeGroup(2, { kind: 'read', data, at: READ_AT }, ctx())
  assert.equal(group.stale, true)
  assert.equal(group.rows.length, 1)
  assert.equal(group.note, 'silent 4 min ago · devices as last heard')
})

test('a node that announced nothing and a node not read are named, never drawn as a node without devices', () => {
  assert.equal(nodeGroup(3, { kind: 'absent', at: 1 }, ctx()).note, NOTHING_ANNOUNCED)
  assert.equal(nodeGroup(3, { kind: 'read', data: inputs(3, []), at: 1 }, ctx()).note, NOTHING_ANNOUNCED)
  assert.equal(nodeGroup(4, { kind: 'failed', at: 1 }, ctx()).note, NOT_READ)
})

test("this node's group says when its events are not shared", () => {
  const data = inputs(0, [device(3, [instance(0)])])
  assert.equal(nodeGroup(0, { kind: 'read', data, at: 1 }, ctx({ share: false })).note, `1 device · 1 instance · ${NOT_SHARED}`)
})

test('the nodes come from this node, the status and the inputs frame, and only moved nodes are read again', () => {
  const frame = [
    { node: 1, source: 'cached' as const, set: 4, age_ms: 0, device_count: 2 },
    { node: 5, source: 'cached' as const, set: 1, age_ms: 0, device_count: 1 },
  ]
  assert.deepEqual(inputNodes(2, null, frame), [1, 2, 5])
  const before = [{ ...frame[0], set: 3 }, frame[1]]
  assert.deepEqual(movedNodes(before, frame), [1])
  assert.deepEqual(movedNodes(null, frame), [1, 5])
  assert.deepEqual(movedNodes(frame, [{ ...frame[1], source: 'stale' }]), [5])
})

test('an answer sent before a newer one never overwrites it', () => {
  const newer = { kind: 'failed' as const, at: 20 }
  assert.equal(newerRead(newer, { kind: 'absent', at: 10 }), newer)
  assert.deepEqual(newerRead(newer, { kind: 'absent', at: 30 }), { kind: 'absent', at: 30 })
  assert.deepEqual(newerRead(undefined, { kind: 'absent', at: 1 }), { kind: 'absent', at: 1 })
})

test('search matches a name or an address on every node, and the totals follow it', () => {
  const g0 = nodeGroup(0, { kind: 'read', data: inputs(0, [device(3, [instance(0), instance(1)])]), at: 1 }, ctx())
  const g1 = nodeGroup(1, { kind: 'read', data: inputs(1, [device(5, [instance(0)], { node: 1, name: 'Dock sensor' })]), at: 1 }, ctx())
  const g3 = nodeGroup(3, { kind: 'absent', at: 1 }, ctx())
  assert.equal(totalsText([g0, g1, g3]), '3 instances on 2 nodes')
  assert.deepEqual(filterGroups([g0, g1, g3], 'dock').map((g) => g.node), [1, 3], 'a node not read stays named')
  assert.deepEqual(filterGroups([g0, g1, g3], 'n0.in3/1').flatMap((g) => g.rows.map((r) => r.address)), ['N0·IN3/1'])
  assert.equal(filterGroups([g0, g1, g3], '').length, 3)
})

test('instance types read in short words, an unread type says so', () => {
  assert.deepEqual(['push_button', 'light_sensor', 'occupancy', null].map(typeName), ['button', 'light', 'occupancy', 'not read'])
})

test('an address search matches whole parts: node 1 is not device 1, IN3 is not IN30', () => {
  const g1 = nodeGroup(1, { kind: 'read', data: inputs(1, [device(5, [instance(0)], { node: 1 })]), at: 1 }, ctx())
  const g2 = nodeGroup(2, { kind: 'read', data: inputs(2, [device(1, [instance(0)], { node: 2 }), device(30, [instance(0)], { node: 2 })]), at: 1 }, ctx())
  assert.deepEqual(filterGroups([g1, g2], 'N1').flatMap((g) => g.rows.map((r) => r.address)), ['N1·IN5/0'])
  assert.deepEqual(filterGroups([g1, g2], 'in3').flatMap((g) => g.rows.map((r) => r.address)), [])
  assert.deepEqual(filterGroups([g1, g2], 'IN30').flatMap((g) => g.rows.map((r) => r.address)), ['N2·IN30/0'])
})

test('devices of two adapters keep their own rows and their own live events', () => {
  const data = inputs(0, [device(3, [instance(0)]), device(3, [instance(0)], { adapter_id: 1, name: 'Second bus panel' })])
  const entry = liveOf({ adapter_id: 1, short_address: 3, instance_number: 0, event: 'press', event_info: 1 }, 0, READ_AT + 1)
  assert.ok(entry)
  const group = nodeGroup(0, { kind: 'read', data, at: READ_AT }, ctx({ live: new Map([entry]) }))
  assert.deepEqual(
    group.rows.map((r) => [r.key, r.address, r.last.kind]),
    [
      ['0:0:3:0', 'N0·IN3/0', 'code'],
      ['0:1:3:0', 'N0·A1·IN3/0', 'named'],
    ],
  )
})

test('the note counts what the node announced, says when the list is cut, and a silent node stays silent', () => {
  const cut = inputs(1, [device(5, [instance(0), instance(1)], { node: 1 })], { device_count: 40 })
  assert.equal(nodeGroup(1, { kind: 'read', data: cut, at: 1 }, ctx()).note, '40 devices · 2 instances · 1 listed')
  const quiet = inputs(2, [], { source: 'stale', age_ms: null })
  assert.equal(nodeGroup(2, { kind: 'read', data: quiet, at: 1 }, ctx()).note, 'silent · devices as last heard')
  const bare = inputs(1, [device(5, [], { node: 1 })])
  assert.equal(totalsText([nodeGroup(1, { kind: 'read', data: bare, at: 1 }, ctx())]), '0 instances on 1 node')
})
