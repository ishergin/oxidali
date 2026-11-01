import assert from 'node:assert/strict'
import test from 'node:test'

import { withGroupToggled } from '../src/screens/poller-groups.js'

const shown = ['runtime_status', 'common_102', 'groups', 'extended']

test('switching a group off removes only that group', () => {
  assert.deepEqual(withGroupToggled(['runtime_status', 'groups'], 'runtime_status', shown), ['groups'])
})

test('switching a group on places it in the order the screen shows', () => {
  assert.deepEqual(withGroupToggled(['groups'], 'common_102', shown), ['common_102', 'groups'])
})

test('a group the screen has no chip for survives toggling the others', () => {
  const stored = ['runtime_status', 'dt7_switching']
  const on = withGroupToggled(stored, 'extended', shown)
  assert.deepEqual(on, ['runtime_status', 'extended', 'dt7_switching'])
  assert.deepEqual(withGroupToggled(on, 'extended', shown), stored)
  assert.deepEqual(withGroupToggled(stored, 'runtime_status', shown), ['dt7_switching'])
})
