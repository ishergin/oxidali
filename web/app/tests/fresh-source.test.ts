import assert from 'node:assert/strict'
import test from 'node:test'

import { freshSource, settledValue } from '../src/fresh-source.js'

const FRESH_MS = 10_000

function harness(results: (() => Promise<string>)[]) {
  let clock = 0
  const previous: (string | null)[] = []
  const source = freshSource(
    (prior: string | null) => {
      previous.push(prior)
      const next = results[previous.length - 1]
      assert.ok(next, 'more loads than scripted')
      return next()
    },
    () => clock,
    FRESH_MS,
  )
  return {
    source,
    previous,
    advance: (ms: number) => {
      clock += ms
    },
  }
}

test('a fresh value is served without asking again', async () => {
  const h = harness([async () => 'a'])
  assert.equal(await h.source(), 'a')
  h.advance(FRESH_MS - 1)
  assert.equal(await h.source(), 'a')
  assert.equal(h.previous.length, 1)
})

test('calls during a load join it instead of starting another', async () => {
  let release: (value: string) => void = () => {}
  const h = harness([() => new Promise<string>((resolve) => (release = resolve))])
  const first = h.source()
  const second = h.source()
  release('a')
  assert.deepEqual(await Promise.all([first, second]), ['a', 'a'])
  assert.equal(h.previous.length, 1)
})

test('a stale value is loaded again, with the previous one at hand', async () => {
  const h = harness([async () => 'a', async () => 'b'])
  await h.source()
  h.advance(FRESH_MS)
  assert.equal(await h.source(), 'b')
  assert.deepEqual(h.previous, [null, 'a'])
})

test('a failed load is not retried inside the fresh interval', async () => {
  const h = harness([async () => Promise.reject(new Error('down')), async () => 'a'])
  await assert.rejects(h.source())
  await assert.rejects(h.source())
  assert.equal(h.previous.length, 1)
  h.advance(FRESH_MS)
  assert.equal(await h.source(), 'a')
})

test('a settled result gives its value, or null when it failed', () => {
  assert.equal(settledValue({ status: 'fulfilled', value: 3 }), 3)
  assert.equal(settledValue({ status: 'rejected', reason: new Error('x') }), null)
})
