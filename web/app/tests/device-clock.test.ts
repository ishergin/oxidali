import assert from 'node:assert/strict'
import test from 'node:test'

import { DEVICE_CLOCK_FLOOR_MS, deviceClock } from '../src/format.js'

test('an anchored stamp renders as a clock', () => {
  const clock = deviceClock(DEVICE_CLOCK_FLOOR_MS + 12 * 3600_000 + 3_723_456)
  assert.equal(clock.anchored, true)
  assert.match(clock.text, /^\d{2}:\d{2}:\d{2}\.\d{3}$/)
})

test('the floor is inclusive, and one millisecond below it is not a time', () => {
  assert.equal(deviceClock(DEVICE_CLOCK_FLOOR_MS).anchored, true)
  assert.equal(deviceClock(DEVICE_CLOCK_FLOOR_MS - 1).anchored, false)
})

test('an unanchored stamp renders as time since boot', () => {
  assert.deepEqual(deviceClock(3158), { text: '+3.158', anchored: false })
  assert.equal(deviceClock(0).text, '+0.000')
  assert.equal(deviceClock(999).text, '+0.999')
})

test('it stays readable past a minute and past an hour', () => {
  assert.equal(deviceClock(63_158).text, '+1:03.158')
  assert.equal(deviceClock(3_723_456).text, '+1:02:03.456')
  assert.equal(deviceClock(36_000_000).text, '+10:00:00.000')
})
