import assert from 'node:assert/strict'
import test from 'node:test'

import { deltaOf, flatten } from '../src/counter-delta.js'
import { deltaView } from '../src/screens/stats-view.js'

function sample(stats: { name: string; count: number }[]) {
  return flatten(deltaView({ sample_ms: 0, rules: { effects_emitted: 0, stats } }), '', {})
}

test('a named count keeps its delta when a new name sorts in before it', () => {
  const before = sample([{ name: 'presses', count: 5 }])
  const after = sample([
    { name: 'away', count: 1 },
    { name: 'presses', count: 7 },
  ])

  assert.equal(deltaOf(after['rules.stats.presses'], before['rules.stats.presses']), 2)
  assert.equal(before['rules.stats.away'], undefined, 'a new name has no delta yet')
})

test('the rest of the sample keeps its paths', () => {
  const flat = sample([{ name: 'presses', count: 5 }])

  assert.equal(flat['sample_ms'], 0)
  assert.equal(flat['rules.effects_emitted'], 0)
  assert.equal(flat['rules.stats.presses'], 5)
})

test('a count that wraps past u32 reads as its increase', () => {
  const before = sample([{ name: 'presses', count: 2 ** 32 - 1 }])
  const after = sample([{ name: 'presses', count: 1 }])

  assert.equal(deltaOf(after['rules.stats.presses'], before['rules.stats.presses']), 2)
})
