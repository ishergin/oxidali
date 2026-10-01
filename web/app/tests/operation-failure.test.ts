import assert from 'node:assert/strict'
import test from 'node:test'

import { opFailure } from '../src/format.js'

test('each read-back refusal names what the operator should check', () => {
  for (const code of ['verify_failed', 'verify_unanswered', 'verify_contended']) {
    const text = opFailure({ status: 'failed', error: { code, message: code } })
    assert.ok(text && text !== code, `${code} gets a hint, not its bare code`)
  }
  assert.match(opFailure({ status: 'failed', error: { code: 'verify_failed' } }) ?? '', /check the device/)
  assert.match(opFailure({ status: 'failed', error: { code: 'verify_contended' } }) ?? '', /duplicate address/)
})

test('any other refusal shows its code and the message the server gave', () => {
  assert.equal(
    opFailure({ status: 'failed', error: { code: 'device_not_found', message: 'no gear at 7' } }),
    'device_not_found — no gear at 7',
  )
  assert.equal(opFailure({ status: 'timed_out', error: { code: 'timeout', message: 'timeout' } }), 'timeout')
})

test('a preemption and an operation without an error have nothing to explain', () => {
  assert.equal(opFailure({ status: 'failed', error: { code: 'preempted' } }), null)
  assert.equal(opFailure({ status: 'succeeded' }), null)
})
