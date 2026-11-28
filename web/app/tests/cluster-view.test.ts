import assert from 'node:assert/strict'
import test from 'node:test'

import type { ClusterNode, ClusterOwn, ClusterSettings, ClusterStatus } from '../src/api/types.js'
import { nextTap } from '../src/components/confirm-tap.js'
import {
  FOREIGN_HOLD,
  NO_DRAFT,
  ageText,
  blockVerdict,
  draftOnRead,
  editDraft,
  executorVerdict,
  forgetLabel,
  forgetRefusal,
  forgettable,
  heardVerdict,
  ignoredReason,
  nodeHref,
  nodeLine,
  pollIsCurrent,
  nodeNumberOptions,
  nodesEmptyText,
  nodeSummary,
  ownVerdict,
  parsePort,
  pinReason,
  refusalOf,
  clipName,
  settingsShown,
  sharesWire,
  unprotectedText,
  unsavedLine,
  wireOptions,
} from '../src/screens/cluster-view.js'
import { fieldPatch, withFieldEdit } from '../src/screens/field-edits.js'

const stored: ClusterSettings = {
  enabled: false,
  cluster_name: '',
  node_number: 0,
  port: 47810,
  wire_same_as: null,
  forward_operator: true,
  forward_home_assistant: true,
  forward_hcl: false,
  forward_foreign_master: false,
  accept_all_lamps: false,
  send_all_lamps: false,
  accept_single_lamps: false,
  wire_protection: true,
  share_input_events: true,
  read_only: false,
  block: { first: 0, last: 15 },
}

const own = (over: Partial<ClusterOwn> = {}): ClusterOwn => ({
  state: 'running',
  node_number: 1,
  wire: 1,
  board: 'dali-9b07e4',
  boot_epoch: 7,
  wire_owner: { node: 1, board: 'dali-9b07e4', this_node: true },
  split_pair: false,
  wire_chain: false,
  unprotected_wire: false,
  number_held_by: null,
  block: 'settled',
  ...over,
})

const node = (n: number, over: Partial<ClusterNode> = {}): ClusterNode => ({
  node: n,
  board: `dali-00000${n}`,
  address: `192.168.13.4${n}`,
  installation: `site-${n}`,
  wire: n,
  wire_protection: true,
  home_assistant_face: null,
  heard: true,
  age_ms: 3000,
  sequence_gaps: 0,
  split: false,
  ...over,
})

const status = (o: ClusterOwn, nodes: ClusterNode[] = []): ClusterStatus => ({ own: o, nodes, ignored: [] })

test('the Apply bar counts only the fields that differ from the controller, and names them in page order', () => {
  let edits = withFieldEdit({}, { port: 47900 }, stored)
  edits = withFieldEdit(edits ?? {}, { enabled: true }, stored)
  edits = withFieldEdit(edits ?? {}, { cluster_name: 'hall' }, stored)
  assert.equal(unsavedLine(fieldPatch(edits ?? {}, stored)), '3 unsaved changes · Enabled, Cluster name, Port')
})

test('an edit taken back is no longer counted, and none at all reads as no unsaved changes', () => {
  let edits = withFieldEdit({}, { forward_hcl: true }, stored)
  edits = withFieldEdit(edits ?? {}, { wire_same_as: 2 }, stored)
  assert.equal(unsavedLine(fieldPatch(edits ?? {}, stored)), '2 unsaved changes · Forward HCL, DALI bus')
  edits = withFieldEdit(edits ?? {}, { forward_hcl: false }, stored)
  assert.equal(unsavedLine(fieldPatch(edits ?? {}, stored)), '1 unsaved change · DALI bus')
  edits = withFieldEdit(edits ?? {}, { wire_same_as: null }, stored)
  assert.equal(edits, null)
  assert.equal(unsavedLine({}), 'No unsaved changes')
})

test('each conflict the controller names lands on its own field, in words', () => {
  const cases: [string, string, RegExp][] = [
    ['cluster_name_required', 'cluster_name', /needs a name/],
    ['wire_names_this_node', 'wire_same_as', /own bus/],
    ['wire_protection_required', 'wire_protection', /foreign master/],
    ['cluster_node_number_pinned', 'node_number', /home of channels or scenes/],
  ]
  for (const [message, field, words] of cases) {
    const r = refusalOf(409, 'conflict', message)
    assert.equal(r.field, field, message)
    assert.match(r.text, words, message)
  }
})

test('a refusal that names no field stays off the fields', () => {
  assert.equal(refusalOf(409, 'controller_standby', 'controller_standby').field, null)
  assert.match(refusalOf(409, 'controller_standby', 'controller_standby').text, /active board/)
  assert.equal(refusalOf(422, 'invalid_value', 'invalid_value').field, null)
  assert.match(refusalOf(422, 'invalid_value', 'invalid_value').text, /out of range/)
  assert.equal(refusalOf(409, 'conflict', 'something_new').field, null)
  assert.equal(refusalOf(409, 'conflict', 'something_new').text, 'Refused: 409 conflict — something_new')
})

test('a refusal of a record or a part lands at its field, in words', () => {
  const cases: [string, string, RegExp][] = [
    ['cluster_stale_revision', 'record_name', /Renamed elsewhere/],
    ['cluster_record_not_live', 'part', /deleted or created again/],
    ['cluster_record_linked', 'part', /already has a part of this node/],
    ['cluster_number_of_another_node', 'record_number', /block of another node/],
    ['cluster_number_taken', 'record_number', /taken/],
  ]
  for (const [message, field, words] of cases) {
    const r = refusalOf(409, 'conflict', message, 'records')
    assert.equal(r.field, field, message)
    assert.match(r.text, words, message)
  }
})

test('a refused command names its cause in words and no field', () => {
  const off = refusalOf(409, 'conflict', 'cluster_off', 'commands')
  assert.equal(off.field, null)
  assert.match(off.text, /cluster is off/)
  assert.match(refusalOf(409, 'conflict', 'cluster_not_running', 'commands').text, /does not hold its number/)
  assert.match(refusalOf(409, 'conflict', 'cluster_no_address', 'commands').text, /no network address/)
})

test('the standby and out-of-range texts follow the page that was refused', () => {
  assert.match(refusalOf(409, 'controller_standby', '', 'settings').text, /change the settings on the active board/)
  assert.match(refusalOf(409, 'controller_standby', '', 'records').text, /edit channels and scenes on the active board/)
  assert.match(refusalOf(409, 'controller_standby', '', 'commands').text, /switch channels and recall scenes/)
  assert.match(refusalOf(422, 'invalid_value', '', 'records').text, /number 0–127/)
})

test('a record no node knows any more is said in words outside the settings', () => {
  assert.match(refusalOf(404, 'not_found', 'not_found', 'records').text, /No node knows this record/)
  assert.equal(refusalOf(404, 'not_found', 'not_found').text, 'Refused: 404 not_found')
})

test('Forget acts only on the second press of the same node, and a press elsewhere re-arms', () => {
  const first = nextTap<string>(null, '2')
  assert.deepEqual(first, { armed: '2', act: false })
  assert.equal(forgetLabel(first.armed === '2'), 'Sure?')
  assert.deepEqual(nextTap(first.armed, '3'), { armed: '3', act: false })
  assert.deepEqual(nextTap(first.armed, '2'), { armed: null, act: true })
  assert.equal(forgetLabel(false), 'Forget')
})

test('Forget is offered only on a silent node and never on the standby', () => {
  assert.equal(forgettable(node(2, { heard: false }), false), true)
  assert.equal(forgettable(node(2, { heard: true }), false), false)
  assert.equal(forgettable(node(2, { heard: false }), true), false)
})

test('a node heard again is reported in words', () => {
  assert.equal(forgetRefusal(2, 409, 'node_heard'), 'Node 2 is heard again; only a silent node can be forgotten.')
  assert.equal(forgetRefusal(2, 503, 'execution_failed'), 'Node 2 was not forgotten — execution_failed.')
})

test('the node number is pinned with its reason while this node is home to records', () => {
  const info = (channels: number, scenes: number) => ({
    nodes: [
      { node: 0, source: 'own' as const, set: null, age_ms: null, channels, scenes, channel_parts: 0, scene_parts: 0, skipped: 0 },
      { node: 1, source: 'cached' as const, set: 3, age_ms: 10, channels: 5, scenes: 0, channel_parts: 0, scene_parts: 0, skipped: 0 },
    ],
  })
  assert.equal(pinReason(info(0, 0)), null)
  assert.equal(pinReason(null), null)
  assert.match(pinReason(info(3, 3)) ?? '', /home of 3 channels and 3 scenes/)
  assert.match(pinReason(info(1, 0)) ?? '', /home of 1 channel\./)
})

test('the bus select never offers this node itself, but keeps a stored value that names it', () => {
  const values = (current: number | null) => wireOptions(null, 1, current).map((o) => o.value)
  assert.deepEqual(values(null), [null, 0, 2, 3, 4, 5, 6, 7])
  assert.ok(values(1).includes(1))
  const labels = wireOptions(status(own(), [node(2)]), 1, null).map((o) => o.label)
  assert.ok(labels.includes('Same bus as node 2 · dali-000002'))
})

test('the node number select names the board that holds a number', () => {
  const labels = nodeNumberOptions(status(own(), [node(2)]), 1).map((o) => o.label)
  assert.equal(labels[2], '2 · held by dali-000002')
  assert.equal(labels[1], '1')
})

test('the port and the name are kept within what the controller accepts', () => {
  assert.equal(parsePort('47810'), 47810)
  assert.equal(parsePort('0'), null)
  assert.equal(parsePort('65536'), null)
  assert.equal(parsePort('12a'), null)
  assert.equal(clipName(`  ${'ж'.repeat(40)}  `).length, 32)
})

test('a node row says what it reports about itself, with colour only for a fault', () => {
  const quiet = nodeSummary(node(2), own())
  assert.deepEqual(quiet, [{ tone: 'plain', text: 'own bus' }])
  const shared = nodeSummary(node(2, { wire: 1, wire_protection: false, split: true, sequence_gaps: 1 }), own())
  assert.deepEqual(shared, [
    { tone: 'err', text: 'pair split' },
    { tone: 'plain', text: 'same bus as this node' },
    { tone: 'warn', text: 'loop protection off' },
    { tone: 'plain', text: '1 frame missed' },
  ])
})

test('an ignored board says why in words', () => {
  const s = status(own(), [node(2)])
  const row = { board: 'dali-0a11f3', address: '10.0.0.9', node: 2, cluster_name: null, age_ms: 0 }
  assert.deepEqual(ignoredReason({ ...row, reason: 'number_held' }, s), {
    text: 'Says it is node 2, which dali-000002 holds. Give it a free number.',
    name: null,
  })
  assert.equal(ignoredReason({ ...row, node: 1, reason: 'number_held' }, s).text, 'Says it is node 1, which dali-9b07e4 holds. Give it a free number.')
  assert.equal(ignoredReason({ ...row, node: 3, reason: 'number_held' }, s).text, 'Says it is node 3, which another board holds. Give it a free number.')
  assert.deepEqual(ignoredReason({ ...row, reason: 'waiting' }, s), { text: 'Waits for node 2, which another board holds.', name: null })
  assert.deepEqual(ignoredReason({ ...row, reason: 'other_cluster', cluster_name: 'hall-b' }, s), {
    text: 'Another cluster name: ',
    name: 'hall-b',
  })
})

test('who executes on the bus follows the wire owner', () => {
  assert.equal(executorVerdict(own()).text, 'this node executes')
  assert.equal(executorVerdict(own({ wire_owner: { node: 0, board: 'dali-3fa2c1', this_node: false } })).text, 'node 0 executes · dali-3fa2c1')
  assert.equal(executorVerdict(own({ wire_chain: true })).tone, 'warn')
  assert.equal(executorVerdict(own({ state: 'off' })).text, 'cluster off')
})

test('the sidebar shows nothing while the cluster is off or unread', () => {
  assert.equal(nodeLine(null), null)
  assert.equal(nodeLine(status(own({ state: 'off' }))), null)
})

test('the sidebar counts the other nodes heard, and a silent one turns it to warn', () => {
  const all = nodeLine(status(own(), [node(0), node(1), node(2)]))
  assert.deepEqual(all, { board: 'dali-9b07e4', text: 'Node 1 · 2 of 2 heard', tone: 'plain', href: '#/settings/cluster' })
  const silent = nodeLine(status(own(), [node(0, { heard: false }), node(2, { heard: false })]))
  assert.equal(silent?.text, 'Node 1 · 0 of 2 heard')
  assert.equal(silent?.tone, 'warn')
  assert.equal(nodeLine(status(own()))?.text, 'Node 1 · no other node heard')
})

test('the sidebar names the faults of the mock in err', () => {
  const taken = nodeLine(status(own({ state: 'waiting', number_held_by: 'dali-9b07e4', board: 'dali-0a11f3' })))
  assert.deepEqual(taken, { board: 'dali-0a11f3', text: 'Node 1 is taken by dali-9b07e4', tone: 'err', href: '#/settings/cluster' })
  const split = nodeLine(status(own({ split_pair: true })))
  assert.deepEqual(split, { board: 'dali-9b07e4', text: 'Pair split · see Redundancy', tone: 'err', href: '#/settings/redundancy' })
  assert.equal(nodeLine(status(own({ state: 'bind_failed' })))?.tone, 'err')
})

test('the standby and a listening node read as listening, without colour', () => {
  for (const state of ['standby', 'listening'] as const) {
    const line = nodeLine(status(own({ state })))
    assert.equal(line?.text, 'Node 1 · listening')
    assert.equal(line?.tone, 'plain')
  }
  assert.equal(nodeLine(status(own({ state: 'no_address' })))?.tone, 'warn')
})

test('an edit clears any refusal, and nothing is drafted while busy or on the standby', () => {
  const refused = { edits: { enabled: true }, refusal: { field: 'cluster_name' as const, text: 'x' } }
  assert.deepEqual(editDraft(refused, { enabled: false }, stored, false), { edits: null, refusal: null })
  assert.equal(editDraft(refused, { port: 1 }, stored, true), refused)
  assert.equal(editDraft(NO_DRAFT, { port: 1 }, { ...stored, read_only: true }, false), NO_DRAFT)
  assert.deepEqual(editDraft(NO_DRAFT, { port: 1 }, stored, false), { edits: { port: 1 }, refusal: null })
})

test('a read drops the edits the controller caught up with and keeps the rest', () => {
  const draft = { edits: { port: 47900, enabled: true }, refusal: null }
  assert.deepEqual(draftOnRead(draft, { ...stored, port: 47900 }), { edits: { enabled: true }, refusal: null })
  assert.equal(draftOnRead(draft, stored), draft)
})

test('on the standby the stored values are shown, and a pending draft is dropped', () => {
  const standby = { ...stored, read_only: true }
  const draft = { edits: { port: 47900 }, refusal: { field: null, text: 'x' } }
  assert.equal(settingsShown(draft, standby).port, stored.port)
  assert.equal(settingsShown(draft, stored).port, 47900)
  assert.equal(draftOnRead(draft, standby), NO_DRAFT)
  assert.equal(draftOnRead(NO_DRAFT, standby), NO_DRAFT)
})

test('a silent node ages by the time since the read, a heard one shows its own age', () => {
  assert.equal(ageText(9_000), '9 s ago')
  assert.equal(ageText(240_000), '4 min ago')
  assert.equal(ageText(7_200_000), '2 h ago')
  assert.equal(ageText(2 * 86_400_000), '2 d ago')
  assert.equal(ageText(2 ** 30), 'over 12 days ago')
  assert.deepEqual(heardVerdict(node(2, { heard: false, age_ms: 9_000 }), 41_000), { tone: 'warn', text: '50 s ago' })
  assert.deepEqual(heardVerdict(node(2, { heard: false, age_ms: 9_000 }), -800), { tone: 'warn', text: '9 s ago' })
  assert.deepEqual(heardVerdict(node(2, { age_ms: 1_500 }), 41_000), { tone: 'ok', text: '1 s ago' })
})

test('the own status names a port that is not bound, and a waiting node names the holder, in err', () => {
  assert.deepEqual(ownVerdict(own({ state: 'bind_failed' }), 47810), { tone: 'err', text: 'port 47810 not bound — retrying' })
  assert.deepEqual(ownVerdict(own({ state: 'waiting', number_held_by: 'dali-000001' }), 47810), {
    tone: 'err',
    text: 'waiting — node 1 is held by dali-000001',
  })
  assert.equal(ownVerdict(own({ state: 'no_address' }), 47810).tone, 'warn')
  assert.equal(ownVerdict(own(), 47810).tone, 'ok')
})

test('an unread status is not drawn as a cluster that is off', () => {
  assert.equal(nodesEmptyText(null), 'Reading the cluster status…')
  assert.equal(nodesEmptyText(status(own({ state: 'off' }))), 'Nothing is heard while the cluster is off.')
  assert.equal(nodesEmptyText(status(own())), 'No other node heard yet.')
})

test('Open goes to the node by its address', () => {
  assert.equal(nodeHref(node(3)), 'http://192.168.13.43/')
})

test('a poll answer counts only when no status frame was taken while it was in flight', () => {
  assert.equal(pollIsCurrent(4, 4), true)
  assert.equal(pollIsCurrent(4, 5), false)
})

test('a name is clipped by whole characters, never inside one', () => {
  const name = `${'a'.repeat(61)}😀`
  assert.equal(clipName(name), 'a'.repeat(61))
  assert.equal(clipName(`${'a'.repeat(60)}😀`), name.slice(1))
})

test('a contested block reads as a fault on the node line, after a split pair and a taken number', () => {
  assert.deepEqual(
    [nodeLine(status(own({ block: 'contested' }), [node(2)]))?.text, nodeLine(status(own({ block: 'contested' })))?.tone],
    ['Node 1 · another board holds it too', 'err'],
  )
  assert.equal(nodeLine(status(own({ block: 'contested', split_pair: true })))?.text, 'Pair split · see Redundancy')
  assert.match(nodeLine(status(own({ block: 'contested', state: 'waiting' })))?.text ?? '', /is taken by/)
  assert.equal(nodeLine(status(own({ block: 'listening' }), [node(2)]))?.text, 'Node 1 · 1 of 1 heard')
})

test('the two block refusals are said in words, and the share switch is named in the Apply bar', () => {
  assert.match(refusalOf(409, 'conflict', 'cluster_block_unsettled', 'records').text, /still listening to its neighbours/)
  assert.match(refusalOf(409, 'conflict', 'cluster_node_contested', 'records').text, /Another board holds this node's number/)
  const edits = withFieldEdit({}, { share_input_events: false }, stored)
  assert.equal(unsavedLine(fieldPatch(edits ?? {}, stored)), '1 unsaved change · Share input events')
})

test('the block row appears for every state but settled, a contested number in err', () => {
  assert.equal(blockVerdict(own()), null)
  assert.deepEqual(blockVerdict(own({ block: 'listening' })), { tone: 'plain', text: 'listening to the neighbours' })
  assert.deepEqual(blockVerdict(own({ block: 'listening', state: 'no_address' })), { tone: 'plain', text: 'waiting for this node to run' })
  assert.deepEqual(blockVerdict(own({ block: 'contested' })), { tone: 'err', text: 'another board holds this number' })
  assert.match(blockVerdict(own({ block: 'passive', state: 'standby' }))?.text ?? '', /active board/)
  assert.match(blockVerdict(own({ block: 'off', state: 'off' }))?.text ?? '', /as stored/)
})

test('an unprotected shared bus names where loop protection is off, and nothing while the status is clear', () => {
  const neighbours = [node(2, { wire: 1, wire_protection: false }), node(3, { wire: 3, wire_protection: false })]
  assert.equal(unprotectedText(own(), neighbours, true), null)
  assert.equal(
    unprotectedText(own({ unprotected_wire: true }), neighbours, true),
    'Loop protection is off on this shared bus at node 2 (dali-000002). Turn it on there, or stop forwarding the foreign master.',
  )
  assert.equal(
    unprotectedText(own({ unprotected_wire: true }), [node(2, { wire: 1 })], false),
    'Loop protection is off on this shared bus here. Turn it on here, or stop forwarding the foreign master.',
  )
})

test('the hold of the foreign master is said only while this node shares its bus', () => {
  assert.equal(sharesWire(own(), [node(2)], null), false)
  assert.equal(sharesWire(own(), [node(2, { wire: 1 })], null), true)
  assert.equal(sharesWire(undefined, [], 2), true)
  assert.equal(sharesWire(undefined, [], null), false)
  assert.match(FOREIGN_HOLD, /about 70 ms, and dimming about 220 ms/)
})
