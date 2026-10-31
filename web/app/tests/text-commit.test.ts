import assert from 'node:assert/strict'
import test from 'node:test'

import { blurCommits } from '../src/components/text-commit.js'

test('leaving a field without typing commits nothing, so a value the poll moved meanwhile stays', () => {
  assert.equal(blurCommits('5', '5', false), false)
})

test('typing back to the text the field had on focus commits nothing', () => {
  assert.equal(blurCommits('12', '12', false), false)
})

test('a changed text commits, and Escape never does', () => {
  assert.equal(blurCommits('7', '5', false), true)
  assert.equal(blurCommits('7', '5', true), false)
})
