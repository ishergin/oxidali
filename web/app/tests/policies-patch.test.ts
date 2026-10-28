import assert from 'node:assert/strict'
import test from 'node:test'

import { policyPatch } from '../src/screens/policies-patch.js'

const at = (short_address: number) => ({ adapter_id: 0, short_address })

const stored = {
  system_failure_level: 254,
  power_on_level: null,
  apply_on_discovery: false,
  exceptions: [at(3), at(5)],
}

test('an untouched draft sends nothing', () => {
  assert.deepEqual(policyPatch(stored, { ...stored }, stored), {})
})

test('a level-only save leaves the exceptions to the controller', () => {
  const moved = { ...stored, exceptions: [at(3), at(9)] }
  const patch = policyPatch(stored, { ...stored, power_on_level: 100 }, moved)
  assert.deepEqual(patch, { power_on_level: 100 })
})

test('each field travels only when the operator changed it', () => {
  assert.deepEqual(policyPatch(stored, { ...stored, system_failure_level: null }, stored), {
    system_failure_level: null,
  })
  assert.deepEqual(policyPatch(stored, { ...stored, apply_on_discovery: true }, stored), {
    apply_on_discovery: true,
  })
})

test('an exception edit lands on the list the controller holds now', () => {
  const draft = { ...stored, exceptions: [at(1), at(5)] }
  const moved = { ...stored, exceptions: [at(3), at(9)] }
  assert.deepEqual(policyPatch(stored, draft, moved), { exceptions: [at(1), at(9)] })
})
