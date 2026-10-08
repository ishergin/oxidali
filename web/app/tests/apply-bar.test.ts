import assert from 'node:assert/strict'
import test from 'node:test'

import { applyBar, boundLamps } from '../src/screens/apply-bar.js'

const bound = boundLamps([
  { virtual_lamp_id: 1, binding: { physical_short_address: 4 } },
  { virtual_lamp_id: 2, binding: null },
])

test('pending edits are what the bar applies', () => {
  const rows = [{ virtual_lamp_id: 1, differs: true }]
  assert.deepEqual(applyBar(2, rows, bound), { kind: 'edits', count: 2 })
})

test('a bound row that still differs from its gear keeps an apply after the edits are saved', () => {
  const rows = [
    { virtual_lamp_id: 1, differs: true },
    { virtual_lamp_id: 2, differs: true },
  ]
  assert.deepEqual(applyBar(0, rows, bound), { kind: 'retry', count: 1 })
})

test('an unbound lamp has no gear to program, so it offers no apply', () => {
  assert.equal(applyBar(0, [{ virtual_lamp_id: 2, differs: true }], bound), null)
  assert.equal(applyBar(0, [{ virtual_lamp_id: 1, differs: false }], bound), null)
})
