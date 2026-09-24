import assert from 'node:assert/strict'
import test from 'node:test'

import { absorbBusLoadSample, createBusLoadHistory } from '../src/components/bus-load-history.js'

function sample(uptimeMs: number, collisions: number) {
  return {
    uptime_ms: uptimeMs,
    dali_wire: {
      wire_ticks_total: 1,
      load_permille: 100,
      load_own_permille: 50,
      collisions,
    },
  }
}

test('two increasing low-uptime samples confirm a short reboot', () => {
  const history = createBusLoadHistory()
  absorbBusLoadSample(history, sample(60_000, 12))
  absorbBusLoadSample(history, sample(1_000, 0))
  assert.deepEqual(history.ring.map((entry) => entry.uptimeMs), [60_000])

  absorbBusLoadSample(history, sample(3_000, 0))
  absorbBusLoadSample(history, sample(5_000, 1))

  assert.deepEqual(history.ring.map((entry) => entry.uptimeMs), [3_000, 5_000])
  assert.deepEqual(history.ring.map((entry) => entry.collisions), [0, 1])
})

test('one stale fallback sample does not clear the chart', () => {
  const history = createBusLoadHistory()
  absorbBusLoadSample(history, sample(60_000, 4))
  absorbBusLoadSample(history, sample(20_000, 2))
  absorbBusLoadSample(history, sample(62_000, 5))

  assert.deepEqual(history.ring.map((entry) => entry.uptimeMs), [60_000, 62_000])
  assert.equal(history.ring[1].collisions, 1)
})
