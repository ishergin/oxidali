import assert from 'node:assert/strict'
import test from 'node:test'

import { codeOf, lexLine } from '../src/rule-lexis.js'

test('a string runs to the next quote on its line, and # outside strings starts a comment', () => {
  assert.deepEqual(lexLine('do log("a#b") # c "d"'), {
    strings: [{ open: 7, close: 11 }],
    comment: 14,
  })
  assert.deepEqual(lexLine('a("x") b("y")'), {
    strings: [
      { open: 2, close: 4 },
      { open: 9, close: 11 },
    ],
    comment: null,
  })
})

test('an unterminated string runs to the end of its line and hides a #', () => {
  assert.deepEqual(lexLine('do lamp("ку # x'), { strings: [{ open: 8, close: null }], comment: null })
})

test('the code of a line masks string contents and drops the comment, keeping columns', () => {
  assert.equal(codeOf('do log("}") # {'), 'do log(" ") ')
  assert.equal(codeOf('rule "a {" {'), 'rule "   " {')
  assert.equal(codeOf('lamp("x {'), 'lamp("   ')
  assert.equal(codeOf('# rule "x" {'), '')
})
