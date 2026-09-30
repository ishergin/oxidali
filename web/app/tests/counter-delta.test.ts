import assert from 'node:assert/strict'
import test from 'node:test'

import { countsByName, deltaOf, flatten } from '../src/counter-delta.js'

function sample(stats: { name: string; count: number }[]) {
  return flatten({ rules: { stats: countsByName(stats) } }, '', {})
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

test('a count that wraps past u32 reads as its increase', () => {
  const before = sample([{ name: 'presses', count: 2 ** 32 - 1 }])
  const after = sample([{ name: 'presses', count: 1 }])

  assert.equal(deltaOf(after['rules.stats.presses'], before['rules.stats.presses']), 2)
})
