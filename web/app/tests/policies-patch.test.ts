import assert from 'node:assert/strict'
import test from 'node:test'

import { policyOnScreen, policyPatch, sendOnFreshRead, withEdit } from '../src/screens/policies-patch.js'

const at = (short_address: number) => ({ adapter_id: 0, short_address })

const stored = {
  system_failure_level: 254,
  power_on_level: 254,
  apply_on_discovery: false,
  exceptions: [at(3), at(5)],
}

test('an untouched policy sends nothing', () => {
  assert.deepEqual(policyPatch({}, stored), {})
})

test('leaving a field with the value it shows keeps no draft, so the screen follows the controller', () => {
  const edits = withEdit({}, { power_on_level: stored.power_on_level }, stored)
  assert.equal(edits, null)
  assert.equal(policyOnScreen(edits ?? {}, { ...stored, power_on_level: 0 }).power_on_level, 0)
})

test('a draft the controller has caught up with is dropped', () => {
  const level = withEdit({}, { power_on_level: 100 }, stored)
  assert.ok(level)
  assert.equal(withEdit(level, {}, { ...stored, power_on_level: 100 }), null)
  const entry = withEdit({}, { exceptions: [at(3), at(5), at(7)] }, stored)
  assert.ok(entry)
  assert.equal(withEdit(entry, {}, { ...stored, exceptions: [at(3), at(5), at(7)] }), null)
})

test('a field the operator did not change shows what the controller holds now', () => {
  const edits = withEdit({}, { system_failure_level: 100 }, stored)
  assert.ok(edits)
  const elsewhere = { ...stored, power_on_level: 0 }
  assert.equal(policyOnScreen(edits, elsewhere).power_on_level, 0)
  assert.equal(policyOnScreen(edits, elsewhere).system_failure_level, 100)
  assert.deepEqual(policyPatch(edits, elsewhere), { system_failure_level: 100 })
})

test('an edit is measured against the value on screen, not the one first loaded', () => {
  const elsewhere = { ...stored, power_on_level: 0 }
  assert.deepEqual(withEdit({}, { power_on_level: 254 }, elsewhere), { power_on_level: 254 })
  assert.equal(withEdit({ power_on_level: 100 }, { power_on_level: 0 }, elsewhere), null)
})

test('an edit the controller already holds is not sent', () => {
  assert.deepEqual(policyPatch({ power_on_level: 0 }, { ...stored, power_on_level: 0 }), {})
})

test('a level-only save leaves the exceptions to the controller', () => {
  const moved = { ...stored, exceptions: [at(3), at(9)] }
  assert.deepEqual(policyPatch({ power_on_level: 100 }, moved), { power_on_level: 100 })
})

test('an exception edit lands on the list the controller holds now', () => {
  const edits = withEdit({}, { exceptions: [at(1), at(5)] }, stored)
  assert.ok(edits)
  const moved = { ...stored, exceptions: [at(3), at(9)] }
  assert.deepEqual(policyPatch(edits, moved), { exceptions: [at(1), at(9)] })
  assert.deepEqual(policyOnScreen(edits, moved).exceptions, [at(1), at(9)])
})

test('successive exception edits add up', () => {
  const added = withEdit({}, { exceptions: [at(3), at(5), at(7)] }, stored)
  assert.ok(added)
  const both = withEdit(added, { exceptions: [at(5), at(7)] }, stored)
  assert.ok(both)
  const moved = { ...stored, exceptions: [at(3), at(9)] }
  assert.deepEqual(policyPatch(both, moved), { exceptions: [at(7), at(9)] })
  assert.equal(withEdit(both, { exceptions: [at(3), at(5)] }, stored), null)
})

test('a save rebases onto a read taken just before it, not onto the last poll', async () => {
  const edits = withEdit({}, { exceptions: [at(3), at(5), at(7)] }, stored)
  assert.ok(edits)
  const order: string[] = []
  const read = async () => {
    order.push('read')
    return { ...stored, exceptions: [at(3), at(9)] }
  }
  const sent = await sendOnFreshRead(edits, read, async (patch) => {
    order.push('send')
    return patch
  })
  assert.deepEqual(order, ['read', 'send'])
  assert.deepEqual(sent, { exceptions: [at(3), at(7), at(9)] })
})
