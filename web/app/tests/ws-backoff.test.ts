import assert from 'node:assert/strict'
import test from 'node:test'

import { BUSY_RETRY_MS, closeVerdict, nextReconnectDelay } from '../src/api/ws-backoff.js'

test('an unreachable controller climbs the ladder and stays at its top rung', () => {
  assert.deepEqual([0, 1, 2, 3, 4, 9].map((a) => nextReconnectDelay(a, false)),
    [1000, 2000, 4000, 8000, 15000, 15000])
})

test('a capacity refusal waits at least a minute from the first retry on', () => {
  for (const attempt of [0, 1, 4, 20]) {
    assert.equal(nextReconnectDelay(attempt, true), BUSY_RETRY_MS)
  }
})

test('a refusal by Origin is final, a capacity refusal is busy, anything else retries', () => {
  assert.equal(closeVerdict(1008), 'refused')
  assert.equal(closeVerdict(1013), 'busy')
  for (const code of [1000, 1001, 1006, 1011]) assert.equal(closeVerdict(code), 'retry')
})
