import assert from 'node:assert/strict'
import test from 'node:test'

import {
  anchorIndex,
  isAtBottom,
  STICK_EPSILON_PX,
  windowBounds,
} from '../src/screens/follow-scroll.js'

test('the bottom has a tolerance, because sub-pixel rows never land on it exactly', () => {
  assert.equal(isAtBottom(1000, 800, 200), true)
  assert.equal(isAtBottom(1000, 800 - STICK_EPSILON_PX, 200), true)
  assert.equal(isAtBottom(1000, 800 - STICK_EPSILON_PX - 1, 200), false)
})

test('a view shorter than its container is at the bottom', () => {
  assert.equal(isAtBottom(150, 0, 400), true)
})

test('following renders the newest rows and counts nothing as newer', () => {
  const bounds = windowBounds(1000, null, 400)
  assert.deepEqual(bounds, { start: 600, end: 1000, newer: 0 })
})

test('a held view keeps its slice while arrivals accumulate behind it', () => {
  const held = windowBounds(1000, 1000, 400)
  assert.deepEqual(held, { start: 600, end: 1000, newer: 0 })

  const later = windowBounds(1212, 1000, 400)
  assert.deepEqual(later, { start: 600, end: 1000, newer: 212 })
})

test('an anchor past the buffer is clamped, so Clear cannot strand the view', () => {
  const bounds = windowBounds(5, 1000, 400)
  assert.deepEqual(bounds, { start: 0, end: 5, newer: 0 })
})

test('a held window does not walk forward when the buffer is trimmed', () => {
  const seqs = (from: number, count: number) =>
    Array.from({ length: count }, (_, i) => from + i)

  let rows = seqs(1, 50)
  const held = anchorIndex(rows, 40)
  assert.equal(held, 40)
  assert.deepEqual(windowBounds(rows.length, held, 100), { start: 0, end: 40, newer: 10 })

  rows = seqs(11, 50)
  const stillHeld = anchorIndex(rows, 40)
  assert.equal(stillHeld, 30, 'seq 40 is the 30th of rows 11..60')
  assert.deepEqual(windowBounds(rows.length, stillHeld, 100), {
    start: 0,
    end: 30,
    newer: 20,
  })
})

test('an anchor trimmed out of the buffer releases the hold', () => {
  const rows = [80, 81, 82]
  assert.equal(anchorIndex(rows, 40), null)
  assert.deepEqual(windowBounds(rows.length, anchorIndex(rows, 40), 100), {
    start: 0,
    end: 3,
    newer: 0,
  })
})
