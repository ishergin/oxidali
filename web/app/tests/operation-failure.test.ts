import assert from 'node:assert/strict'
import test from 'node:test'

import {
  notKnownToast,
  opEnd,
  opFailure,
  opFailureDetail,
  opStatusChip,
  opStatusLabel,
  preemptedNote,
  startFailureToast,
  startRefused,
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
  for (const status of [400, 404, 409, 422, 499]) assert.equal(startRefused(status, false), true, String(status))
  for (const status of [304, 500, 502, 503, 504, undefined]) {
    assert.equal(startRefused(status, false), false, String(status))
  }
})

test('a 4xx after an earlier attempt was accepted refuses nothing: that attempt may have written', () => {
  for (const status of [409, 422]) assert.equal(startRefused(status, true), false, String(status))
})

const failed = (code: string, message?: string) => ({ status: 'failed', error: { code, message } })

test('a bank 1 write names the field the controller named, with no claim about the whole write', () => {
  for (const code of ['verify_failed', 'verify_unanswered', 'verify_contended']) {
    assert.equal(opFailure(failed(code, 'cri')), `${code} — cri`)
    assert.equal(opFailureDetail(failed(code, 'cri')), 'cri')
  }
})

test('a contended bank 1 step carries its own cause, not a duplicate address', () => {
  for (const [code, subject] of [['read_contended', 'device_identity'], ['verify_contended', 'bank1_lock_byte']]) {
    assert.equal(opFailure(failed(code, subject)), `${code} — ${subject}`)
    assert.doesNotMatch(opFailureDetail(failed(code, subject)) ?? '', /duplicate address/)
  }
})

test('a lock byte not as found says so and that the fields may be held, whatever the code', () => {
  for (const code of ['verify_failed', 'verify_unanswered', 'verify_contended', 'operation_failed']) {
    const detail = opFailureDetail(failed(code, 'bank1_lock_not_restored')) ?? ''
    assert.match(detail, /lock byte did not read back as the write found it/, code)
    assert.match(detail, /the fields may be held — read bank 1/, code)
    assert.doesNotMatch(detail, /left open/, code)
    assert.equal(opFailure(failed(code, 'bank1_lock_not_restored')), `${code} — ${detail}`)
  }
})

test('an unfinished bank 1 read-back says the writes went out and none is proved', () => {
  for (const code of ['verify_unanswered', 'verify_contended', 'operation_failed']) {
    const text = opFailure(failed(code, 'bank1_readback')) ?? ''
    assert.match(text, /the writes went out, but the read-back of bank 1 did not finish, so none is proved/, code)
    assert.doesNotMatch(text, /did not take|unconfirmed/, code)
  }
})

test('a failure detail never repeats its code, because the operation card prints it beside', () => {
  for (const [code, message] of [['verify_failed', 'cri'], ['verify_failed', 'bank1_lock_not_restored'], ['verify_failed', undefined]]) {
    assert.doesNotMatch(opFailureDetail(failed(code as string, message)) ?? '', /verify_failed/)
  }
})

test('only succeeded and failed are known ends; anything else is not known', () => {
  assert.equal(opEnd({ status: 'succeeded' }), 'succeeded')
  assert.equal(opEnd({ status: 'failed' }), 'failed')
  for (const status of ['timed_out', 'cancelled', 'unknown', 'running', 'accepted']) {
    assert.equal(opEnd({ status }), 'not_known', status)
  }
  assert.equal(opEnd(null), 'not_known')
})

test('a refused start says failed; any other start failure says the status is not known', () => {
  assert.deepEqual(startFailureToast({ status: 409, text: '409 conflict' }, false), {
    status: 'failed',
    detail: '409 conflict',
  })
  for (const [status, retried] of [[409, true], [503, false], [undefined, false]] as const) {
    assert.deepEqual(startFailureToast({ status, text: 'x' }, retried), notKnownToast('x'), `${status} ${retried}`)
  }
})

test('only a read that stood down claims it changed nothing; any other type says what it did is not undone', () => {
  for (const type of ['attribute_read', 'memory_bank_read']) {
    assert.equal(preemptedNote({ type }), 'Stood down for an operator command — nothing was changed.', type)
  }
  for (const type of ['attribute_write', 'discovery', 'commissioning_identify', 'group_apply', 'a_future_type']) {
    const note = preemptedNote({ type })
    assert.doesNotMatch(note, /nothing was changed/, type)
    assert.doesNotMatch(note, /read the device/, type)
    assert.equal(note, 'Stood down for an operator command; whatever it did before that is not undone.', type)
  }
})
