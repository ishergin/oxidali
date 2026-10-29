import assert from 'node:assert/strict'
import test from 'node:test'

import {
  requestRefused,
  notKnownToast,
  opFailure,
  opFailureDetail,
  opStatusChip,
  opStatusLabel,
} from '../src/format.js'

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

test('the operation detail names the hint or the message beside the code, never the code twice', () => {
  assert.match(opFailureDetail({ status: 'failed', error: { code: 'verify_failed', message: 'verify_failed' } }) ?? '', /check the device/)
  assert.match(opFailureDetail({ status: 'failed', error: { code: 'read_contended', message: 'read_contended' } }) ?? '', /duplicate address/)
  assert.equal(opFailureDetail({ status: 'failed', error: { code: 'device_not_found', message: 'no gear at 7' } }), 'no gear at 7')
  assert.equal(opFailureDetail({ status: 'failed', error: { code: 'timeout', message: 'timeout' } }), null)
})

test('a preemption and an operation without an error have nothing to explain', () => {
  assert.equal(opFailure({ status: 'failed', error: { code: 'preempted' } }), null)
  assert.equal(opFailure({ status: 'succeeded' }), null)
  assert.equal(opFailureDetail({ status: 'failed', error: { code: 'preempted' } }), null)
})

test('a toast whose follow lost the operation says the status is not known, never failed', () => {
  const toast = notKnownToast('Failed to fetch')
  assert.notEqual(toast.status, 'failed')
  assert.equal(opStatusLabel(toast), 'unknown')
  assert.notEqual(opStatusChip(toast.status).cls, 'err')
  assert.equal(toast.detail, 'status not known — Failed to fetch')
})

test('only a 4xx answer refuses a start; no answer or a 5xx leaves its outcome not known', () => {
  for (const status of [400, 404, 409, 422, 499]) assert.equal(requestRefused(status), true, String(status))
  for (const status of [500, 502, 503, 504, undefined]) assert.equal(requestRefused(status), false, String(status))
})
