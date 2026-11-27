import assert from 'node:assert/strict'
import test from 'node:test'

import type { ClusterNodePart, ClusterRecord } from '../src/api/types.js'
import { nextTap } from '../src/components/confirm-tap.js'
import {
  EMPTY_DRAFT,
  type ApplyPorts,
  type OwnParts,
  type Records,
  deleteKey,
  draftAfter,
  draftItems,
  draftLine,
  draftOnRead,
  editName,
  editPart,
  editsOf,
  fieldErrors,
  finishKey,
  homeCell,
  homeName,
  holderOf,
  ignoringNodes,
  levelBody,
  localOptions,
  nodesTakingPart,
  orphanName,
  orphansOf,
  parseRecordNumber,
  planApply,
  presetPlan,
  recallsOn,
  recordKey,
  runApply,
  runPreset,
} from '../src/screens/cluster-records.js'

function rec(number: number, over: Partial<ClusterRecord> = {}): ClusterRecord {
  return {
    number,
    home_node: Math.floor(number / 16),
    birth_mark: { generation: 1, nonce: 100 + number },
    name: `Channel ${number}`,
    revision: 1,
    source: 'own',
    ...over,
  }
}

function part(r: ClusterRecord, local: number, over: Partial<ClusterNodePart> = {}): ClusterNodePart {
  return {
    adapter: 0,
    group: local,
    record: { number: r.number, birth_mark: r.birth_mark },
    send: true,
    accept: false,
    revision: 1,
    node: 0,
    source: 'own',
    link: 'live',
    ...over,
  }
}

const CH2 = rec(2, { name: 'Corridor' })
const CH5 = rec(5, { name: 'Hall' })
const CS0 = rec(0, { name: 'Evening' })

function world(channelOwn: ClusterNodePart[] = [], sceneOwn: ClusterNodePart[] = []) {
  const records: Records = { channel: [CH2, CH5], scene: [CS0] }
  const own: OwnParts = { channel: channelOwn, scene: sceneOwn }
  return { records, own }
}

test('edits across channels and scenes gather in one counted draft', () => {
  const { own } = world([part(CH2, 3)])
  let d = editName(EMPTY_DRAFT, 'channel', CH2, 'Corridor east')
  d = editPart(d, 'channel', CH2, own.channel, { local: 5 })
  d = editPart(d, 'scene', CS0, own.scene, { local: 1 })
  assert.equal(draftLine(draftItems(d)), '3 unsaved changes · CH2 name, CH2 part, CS0 part')
  assert.equal(editsOf(d, recordKey('channel', CH2)), 2)
})

test('an edit put back to the server value leaves the draft', () => {
  const { own } = world([part(CH2, 3)])
  let d = editPart(EMPTY_DRAFT, 'channel', CH2, own.channel, { local: 5 })
  d = editPart(d, 'channel', CH2, own.channel, { local: 3 })
  d = editName(d, 'channel', CH2, 'Corridor')
  assert.equal(draftLine(draftItems(d)), 'No unsaved changes')
})

test('the name edit keeps the revision read when it began', () => {
  let d = editName(EMPTY_DRAFT, 'channel', CH2, 'A')
  d = editName(d, 'channel', { ...CH2, revision: 9 }, 'AB')
  assert.equal(d.names[recordKey('channel', CH2)].revision, 1)
})

test('a read the controller caught up with drops the edit; others stay', () => {
  const { records, own } = world([part(CH2, 3)])
  let d = editName(EMPTY_DRAFT, 'channel', CH2, 'Corridor east')
  d = editPart(d, 'channel', CH5, own.channel, { local: 7 })
  const after: Records = { ...records, channel: [{ ...CH2, name: 'Corridor east', revision: 2 }, CH5] }
  const kept = draftOnRead(d, after, own)
  assert.deepEqual(Object.keys(kept.names), [])
  assert.deepEqual(Object.keys(kept.parts), [recordKey('channel', CH5)])
})

test('a deleted record takes its draft with it', () => {
  const { own } = world()
  const d = editName(EMPTY_DRAFT, 'channel', CH5, 'Hall 2')
  assert.deepEqual(draftOnRead(d, { channel: [CH2], scene: [CS0] }, own).names, {})
})

test('apply sends renames, then every unlink, then every link', () => {
  const { records, own } = world([part(CH2, 3), part(CH5, 4)])
  let d = editName(EMPTY_DRAFT, 'channel', CH2, 'Corridor east')
  d = editPart(d, 'channel', CH2, own.channel, { local: 4 })
  d = editPart(d, 'channel', CH5, own.channel, { local: 3 })
  const ops = planApply(d, records, own).map((s) => `${s.op}:${s.item}`)
  assert.deepEqual(ops, [
    'rename:CH2 name',
    'unlink:CH2 part',
    'unlink:CH5 part',
    'link:CH2 part',
    'link:CH5 part',
  ])
})

test('a group already in another part is refused on the page, before anything is sent', () => {
  const { records, own } = world([part(CH2, 3)])
  const d = editPart(EMPTY_DRAFT, 'channel', CH5, own.channel, { local: 3 })
  const [step] = planApply(d, records, own)
  assert.equal(step.op, 'refuse')
  assert.match(step.op === 'refuse' ? step.text : '', /G3 is the part of CH2/)
})

test('a group the draft frees can be taken by another record in the same draft', () => {
  const { own } = world([part(CH2, 3)])
  const d = editPart(EMPTY_DRAFT, 'channel', CH2, own.channel, { local: null })
  assert.equal(holderOf('channel', 3, own.channel, d, recordKey('channel', CH5)), null)
  const twice = editPart(d, 'channel', CH5, own.channel, { local: 3 })
  assert.equal(holderOf('channel', 3, own.channel, twice, recordKey('channel', CH2)), 'CH5')
})

test('a left-over part still holds its group', () => {
  const gone = rec(0, { birth_mark: { generation: 1, nonce: 7 } })
  const own = [part(gone, 7, { link: 'orphan' })]
  assert.equal(holderOf('channel', 7, own, EMPTY_DRAFT, recordKey('channel', CH2)), 'CH0 · left over')
  const labels = localOptions('channel', [{ id: 7, name: '', members: 1 }], own, EMPTY_DRAFT, recordKey('channel', CH2))
  assert.deepEqual(
    labels.map((o) => o.label),
    ['Not linked', 'G7 · part of CH0 · left over'],
  )
})

test('each request reports its own outcome and a refused unlink holds back its link', async () => {
  const { records, own } = world([part(CH2, 3), part(CH5, 4)])
  let d = editName(EMPTY_DRAFT, 'channel', CH5, 'Hall 2')
  d = editPart(d, 'channel', CH2, own.channel, { local: 4 })
  d = editPart(d, 'channel', CH5, own.channel, { local: null })
  const sent: string[] = []
  const ports: ApplyPorts = {
    rename: async () => {
      sent.push('rename')
      throw Object.assign(new Error('cluster_stale_revision'), { status: 409, code: 'conflict' })
    },
    unlink: async (_k, local) => {
      sent.push(`unlink G${local}`)
      if (local === 4) throw new Error('boom')
    },
    link: async (_k, local) => {
      sent.push(`link G${local}`)
    },
    refusal: (e) => (e as Error).message,
  }
  const out = await runApply(planApply(d, records, own), ports)
  assert.deepEqual(sent, ['rename', 'unlink G3', 'unlink G4'])
  assert.deepEqual(
    out.map((o) => [o.item, o.ok]),
    [
      ['CH5 name', false],
      ['CH2 part', true],
      ['CH5 part', false],
      ['CH2 part', false],
    ],
  )
  assert.match(out[3].text, /G4 was not linked/)
})

test('a new link starts with Send on and Accept off', () => {
  const { records, own } = world()
  const d = editPart(EMPTY_DRAFT, 'channel', CH5, own.channel, { local: 6 })
  const [step] = planApply(d, records, own)
  assert.equal(step.op, 'link')
  assert.deepEqual(step.op === 'link' ? step.body : null, {
    record: { number: 5, birth_mark: CH5.birth_mark },
    send: true,
    accept: false,
  })
})

test('a number in another node’s block is refused with that node', () => {
  assert.deepEqual(parseRecordNumber('channel', '20', 0), {
    ok: false,
    text: 'CH20 is in the block of node 1: create it on node 1.',
    node: 1,
  })
  assert.deepEqual(parseRecordNumber('channel', 'CH3', 0), { ok: true, number: 3 })
  assert.deepEqual(parseRecordNumber('scene', '', 2), { ok: true, number: null })
  assert.equal(parseRecordNumber('channel', '128', 0).ok, false)
  assert.equal(parseRecordNumber('channel', '1.5', 0).ok, false)
})

test('own channels: group N becomes 16·node + N; existing linked, taken skipped', () => {
  const ch17 = rec(17, { home_node: 1 })
  const ch22 = rec(22, { home_node: 1 })
  const ch18 = rec(18, { home_node: 1 })
  const own = [part(ch18, 2), part(ch22, 8)]
  const locals = [
    { id: 1, name: 'Kitchen', members: 3 },
    { id: 2, name: 'Desk', members: 2 },
    { id: 4, name: '', members: 1 },
    { id: 6, name: 'Bath', members: 2 },
  ]
  const plan = presetPlan('channel', 'own', locals, [ch17, ch18, ch22], own, 1)
  assert.deepEqual(
    plan.map((r) => [r.action, r.number, r.text]),
    [
      ['link', 17, 'CH17 exists: link G1 to it'],
      ['skip', 18, 'skipped: G2 is the part of CH18'],
      ['create', 20, 'create CH20 and link G4'],
      ['skip', 22, 'skipped: CH22 already has G8 of this node'],
    ],
  )
})

test('same numbers: only node 0 creates the missing records', () => {
  const locals = [{ id: 2, name: 'Hall', members: 1 }, { id: 7, name: '', members: 1 }]
  const onOne = presetPlan('channel', 'same', locals, [CH2], [], 1)
  assert.deepEqual(
    onOne.map((r) => [r.action, r.text]),
    [
      ['link', 'CH2 exists: link G2 to it'],
      ['skip', 'CH7 does not exist; only node 0 creates channels 0–15'],
    ],
  )
  const onZero = presetPlan('scene', 'same', [{ id: 7, name: 'Off', members: 0 }], [], [], 0)
  assert.deepEqual(onZero.map((r) => r.text), ['create CS7 “Off” and link S7'])
})

test('a preset reports per group and links the record the controller created', async () => {
  const plan = presetPlan('channel', 'same', [{ id: 1, name: 'A', members: 1 }, { id: 2, name: 'B', members: 1 }], [CH2], [], 0)
  const created = rec(1, { birth_mark: { generation: 3, nonce: 42 } })
  const linked: unknown[] = []
  let tries = 0
  const out = await runPreset('channel', plan, [CH2], {
    create: async () => {
      tries += 1
      if (tries === 1) throw Object.assign(new Error('cluster_nonce_taken'), { status: 409, code: 'conflict' })
      return created
    },
    link: async (_k, local, body) => {
      linked.push([local, body.record])
      if (local === 2) throw new Error('refused here')
    },
    refusal: (e) => (e as Error).message,
  })
  assert.equal(tries, 2)
  assert.deepEqual(linked[0], [1, { number: 1, birth_mark: { generation: 3, nonce: 42 } }])
  assert.deepEqual(
    out.map((o) => [o.state, o.text]),
    [
      ['done', 'created CH1 · G1 linked to CH1'],
      ['refused', 'refused here'],
    ],
  )
})

test('left-over parts are listed and named', () => {
  const gone = rec(0, { birth_mark: { generation: 1, nonce: 7 } })
  const own = [part(CH2, 3), part(gone, 7, { link: 'orphan' })]
  const orphans = orphansOf(own)
  assert.equal(orphans.length, 1)
  assert.equal(orphanName('channel', orphans[0], [CH2]), 'deleted channel')
  assert.equal(orphanName('channel', orphans[0], [rec(0)]), 'an earlier CH0')
})

test('nodes, silent nodes and nodes that ignore commands are read from parts', () => {
  const parts = [
    part(CH5, 1, { node: 0 }),
    part(CH5, 6, { node: 1, accept: false, source: 'cached' }),
    part(CH5, 2, { node: 2, accept: true, source: 'stale' }),
    part(CH5, 3, { node: 3, link: 'orphan' }),
  ]
  assert.deepEqual(nodesTakingPart(5, parts), [
    { node: 0, silent: false },
    { node: 1, silent: false },
    { node: 2, silent: true },
  ])
  assert.deepEqual(ignoringNodes(5, parts, 0), [1])
  const scenes = [{ ...part(CS0, 0, { node: 1 }), group: undefined, scene: 2 }, { ...part(CS0, 0), group: undefined, scene: 1 }]
  assert.equal(recallsOn('scene', 0, scenes), 'N0 S1 · N1 S2')
})

test('the level control sends a level, and 0 is Off', () => {
  assert.deepEqual(levelBody(0), { power: 'off' })
  assert.deepEqual(levelBody(128), { power: 'on', level: 128 })
  assert.deepEqual(levelBody(999), { power: 'on', level: 254 })
})

test('a record that keeps its group is not told the group is taken by a draft claim', () => {
  const { records, own } = world([part(CH2, 5)])
  const d = editPart(EMPTY_DRAFT, 'channel', CH5, own.channel, { local: 5 })
  assert.equal(holderOf('channel', 5, own.channel, d, recordKey('channel', CH2)), null)
  assert.equal(holderOf('channel', 5, own.channel, d, recordKey('channel', CH5)), 'CH2')
  assert.deepEqual(
    planApply(d, records, own).map((s) => s.op),
    ['refuse'],
  )
})

test('after Apply a success leaves the draft, a refusal stays at its field, a stale name is dropped', async () => {
  const { records, own } = world([part(CH2, 3)])
  let d = editName(EMPTY_DRAFT, 'channel', CH2, 'Corridor east')
  d = editName(d, 'channel', CH5, 'Hall 2')
  d = editPart(d, 'channel', CH5, own.channel, { local: 6 })
  const out = await runApply(planApply(d, records, own), {
    rename: async (_k, n) => {
      if (n === 5) throw Object.assign(new Error('cluster_stale_revision'), { status: 409, code: 'conflict' })
    },
    unlink: async () => undefined,
    link: async () => {
      throw Object.assign(new Error('cluster_record_not_live'), { status: 409, code: 'conflict' })
    },
    refusal: (e) => `refused: ${(e as Error).message}`,
  })
  const after = draftAfter(d, out)
  assert.deepEqual(Object.keys(after.names), [])
  assert.deepEqual(Object.keys(after.parts), [recordKey('channel', CH5)])
  assert.deepEqual(fieldErrors(out), {
    [recordKey('channel', CH5)]: { name: 'refused: cluster_stale_revision', part: 'refused: cluster_record_not_live' },
  })
})

test('an empty name is refused on the page and kept in the draft', () => {
  const { records, own } = world()
  const d = editName(EMPTY_DRAFT, 'channel', CH2, '   ')
  const [step] = planApply(d, records, own)
  assert.equal(step.op, 'refuse')
  assert.equal(step.item, 'CH2 name')
})

test('Delete and Finish delete act only on a second press of the same button', () => {
  const del = deleteKey('channel', CH2)
  const fin = finishKey('channel', part(rec(0, { birth_mark: { generation: 1, nonce: 7 } }), 7, { link: 'orphan' }))
  assert.equal(del, `del:${recordKey('channel', CH2)}`)
  assert.equal(fin, 'fin:channel:0:7')
  const first = nextTap<string>(null, del)
  assert.deepEqual(first, { armed: del, act: false })
  assert.deepEqual(nextTap<string>(first.armed, fin), { armed: fin, act: false })
  assert.deepEqual(nextTap<string>(first.armed, del), { armed: null, act: true })
  assert.notEqual(deleteKey('channel', CH2), deleteKey('scene', CS0))
})

test('a record is renamed and deleted only on its home node; elsewhere the row names that node', () => {
  assert.deepEqual(homeName('channel', CH2), { editable: true, text: 'kept on this node, the home of CH2', openNode: null })
  assert.equal(homeCell(CH2), 'this node')
  const cached = rec(19, { source: 'cached', home_node: 1 })
  assert.deepEqual(homeName('channel', cached), {
    editable: false,
    text: 'kept on node 1, the home of CH19: renamed and deleted there',
    openNode: 1,
  })
  assert.equal(homeCell(cached), 'node 1')
  const stale = rec(37, { source: 'stale', home_node: 2 })
  assert.deepEqual(homeName('channel', stale), {
    editable: false,
    text: 'kept on node 2 · cannot be renamed or deleted until node 2 answers',
    openNode: null,
  })
})

test('on the standby a read empties the draft', () => {
  const { records, own } = world([part(CH2, 3)])
  const d = editName(EMPTY_DRAFT, 'channel', CH2, 'Corridor east')
  assert.equal(draftOnRead(d, records, own, true), EMPTY_DRAFT)
  assert.notEqual(draftOnRead(d, records, own, false), EMPTY_DRAFT)
})
