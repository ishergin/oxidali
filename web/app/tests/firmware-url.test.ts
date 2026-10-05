import assert from 'node:assert/strict'
import test from 'node:test'

import { urlProblem } from '../src/screens/firmware-url.js'

test('only http and https are offered to the controller', () => {
  assert.equal(urlProblem('ftp://h/x.bin', 96), 'Must start with http:// or https://')
  assert.equal(urlProblem('http://h/x.bin', 96), null)
})

test('the limit counts bytes, as the server does, not characters', () => {
  const ascii = `http://h/${'a'.repeat(87)}`
  assert.equal(ascii.length, 96)
  assert.equal(urlProblem(ascii, 96), null)
  const cyrillic = `http://h/${'я'.repeat(44)}`
  assert.ok(cyrillic.length <= 96, 'fits the limit in characters')
  assert.equal(urlProblem(cyrillic, 96), 'Too long: 97 of 96 bytes')
})
