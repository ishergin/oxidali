import assert from 'node:assert/strict'
import test from 'node:test'

import { dropLocked, gearDiffers, groupLocks, lockHint, lockHref, lockLabel } from '../src/screens/groups-view.js'

test('a row differs from its gear when any group does', () => {
  assert.equal(gearDiffers({ desired: [true, false, true], applied: [true, false, true] }), false)
  assert.equal(gearDiffers({ desired: [true, true, false], applied: [true, false, true] }), true)
})

const mark = (nonce: number) => ({ generation: 1, nonce })

test('a group in a channel part is locked and labelled by its link: live, left over, or not heard', () => {
  const parts = [
    { adapter: 0, group: 3, record: { number: 2, birth_mark: mark(7) }, link: 'live' as const },
    { adapter: 0, group: 5, record: { number: 0, birth_mark: mark(9) }, link: 'orphan' as const },
    { adapter: 0, group: 8, record: { number: 40, birth_mark: mark(4) }, link: 'unknown' as const },
    { adapter: 1, group: 6, record: { number: 2, birth_mark: mark(7) }, link: 'live' as const },
  ]
  const locks = groupLocks(0, parts, [{ number: 2, name: 'Corridor' }, { number: 0, name: 'New' }])
  assert.deepEqual([...locks.keys()], [3, 5, 8])
  assert.equal(lockLabel(locks.get(3)!), 'CH2 · Corridor')
  assert.equal(lockHref(locks.get(3)!), '#/cluster/channels/2')
  assert.equal(lockLabel(locks.get(5)!), 'CH0 · left over')
  assert.equal(lockHref(locks.get(5)!), '#/cluster')
  assert.equal(lockLabel(locks.get(8)!), 'CH40 · not heard')
  assert.equal(lockHref(locks.get(8)!), '#/cluster')
})

test('the hint names every locked group and says the lamps wait for the channel editor', () => {
  assert.equal(lockHint(new Map()), null)
  const one = lockHint(new Map([[3, { channel: 2, name: 'Corridor', link: 'live' as const }]]))!
  assert.match(one, /^G3 is linked to cluster channel CH2 · Corridor, and a group/)
  assert.match(one, /its column is closed here/)
  const two = lockHint(
    new Map([
      [5, { channel: 5, name: 'Hall', link: 'live' as const }],
      [3, { channel: 2, name: 'Corridor', link: 'live' as const }],
    ]),
  )!
  assert.match(two, /^G3 is linked to cluster channel CH2 · Corridor and G5 to CH5 · Hall, and/)
  assert.match(two, /lamp editor is not built yet/)
})

test('an unsaved tick in a column that closes leaves the draft; others stay', () => {
  const locks = new Map([[3, { channel: 2, name: 'Corridor', link: 'live' as const }]])
  const edits = new Map([
    ['0:3', true],
    ['1:3', false],
    ['0:4', true],
  ])
  assert.deepEqual([...dropLocked(edits, locks).keys()], ['0:4'])
  const open = new Map([['0:4', true]])
  assert.equal(dropLocked(open, locks), open)
})
