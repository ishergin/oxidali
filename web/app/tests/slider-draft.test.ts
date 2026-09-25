import assert from 'node:assert/strict'
import test from 'node:test'

import { draftAfterPoll, draftAfterRefusal } from '../src/slider-draft.js'

test('a poll that has not caught up keeps the draft', () => {
  assert.equal(draftAfterPoll(230, 178), 230)
  assert.equal(draftAfterPoll(230, null), 230)
  assert.equal(draftAfterPoll(230, undefined), 230)
})

test('a poll that shows the draft ends it', () => {
  assert.equal(draftAfterPoll(230, 230), null)
  assert.equal(draftAfterPoll(null, 178), null)
})

test('a refused write drops the draft it wrote', () => {
  assert.equal(draftAfterRefusal(230, 230), null)
})

test('a refusal of an older write leaves a newer drag alone', () => {
  assert.equal(draftAfterRefusal(120, 230), 120)
  assert.equal(draftAfterRefusal(null, 230), null)
})
