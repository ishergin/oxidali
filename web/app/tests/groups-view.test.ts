import assert from 'node:assert/strict'
import test from 'node:test'

import { gearDiffers, groupLocks, lockHint, lockHref, lockLabel } from '../src/screens/groups-view.js'

test('a row differs from its gear when any group does', () => {
  assert.equal(gearDiffers({ desired: [true, false, true], applied: [true, false, true] }), false)
  assert.equal(gearDiffers({ desired: [true, true, false], applied: [true, false, true] }), true)
})

const mark = (nonce: number) => ({ generation: 1, nonce })

test('a group in a channel part is locked and labelled with its channel; a left-over part too', () => {
  const parts = [
    { adapter: 0, group: 3, record: { number: 2, birth_mark: mark(7) } },
    { adapter: 0, group: 5, record: { number: 0, birth_mark: mark(9) } },
    { adapter: 1, group: 6, record: { number: 2, birth_mark: mark(7) } },
  ]
  const locks = groupLocks(0, parts, [{ number: 2, birth_mark: mark(7), name: 'Corridor' }, { number: 0, birth_mark: mark(1), name: 'New' }])
  assert.deepEqual([...locks.keys()], [3, 5])
  assert.equal(lockLabel(locks.get(3)!), 'CH2 · Corridor')
  assert.equal(lockHref(locks.get(3)!), '#/cluster/channels/2')
  assert.equal(lockLabel(locks.get(5)!), 'CH0 · left over')
  assert.equal(lockHref(locks.get(5)!), '#/cluster')
})

test('the hint names every locked group and says the lamps wait for the channel editor', () => {
  assert.equal(lockHint(new Map()), null)
  const one = lockHint(new Map([[3, { channel: 2, name: 'Corridor', leftOver: false }]]))!
  assert.match(one, /^G3 is linked to cluster channel CH2 · Corridor, and a group/)
  assert.match(one, /its column is closed here/)
  const two = lockHint(
    new Map([
      [5, { channel: 5, name: 'Hall', leftOver: false }],
      [3, { channel: 2, name: 'Corridor', leftOver: false }],
    ]),
  )!
  assert.match(two, /^G3 is linked to cluster channel CH2 · Corridor and G5 to CH5 · Hall, and/)
  assert.match(two, /lamp editor is not built yet/)
})
