import assert from 'node:assert/strict'
import test from 'node:test'

import { gearDiffers } from '../src/screens/groups-view.js'

test('a row differs from its gear when any group does', () => {
  assert.equal(gearDiffers({ desired: [true, false, true], applied: [true, false, true] }), false)
  assert.equal(gearDiffers({ desired: [true, true, false], applied: [true, false, true] }), true)
})
