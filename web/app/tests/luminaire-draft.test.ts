import assert from 'node:assert/strict'
import test from 'node:test'

import {
  canWrite,
  draftNotes,
  EMPTY_DRAFT,
  ONE_TEXT_NOTE,
  OUTCOME_NOT_KNOWN,
  patchBody,
  touched,
  WHOLE_NUMBER_NOTE,
  wholeNumber,
  writeOutcome,
  type WriteStarts,
} from '../src/screens/luminaire-draft.js'

const NO_START: WriteStarts = { accepted: false, refusal: null }
const refusedStart = (status: number | undefined, text: string): WriteStarts => ({
  accepted: false,
  refusal: { status, text },
})

test('a number is a whole, non-negative decimal and nothing else', () => {
  assert.equal(wholeNumber('81'), 81)
  assert.equal(wholeNumber('0'), 0)
  for (const raw of ['81.0', '0x51', '8,1', '-1', '', ' 81', '1e2']) {
    assert.equal(wholeNumber(raw), null, raw)
  }
})

test('Write sends every touched field with its typed value, even one equal to the screen', () => {
  let draft = touched(EMPTY_DRAFT, 'numbers', 'cri', '81')
  draft = touched(draft, 'numbers', 'nominal_input_power_w', '35')
  draft = touched(draft, 'texts', 'luminaire_colour', 'warm white')
  assert.deepEqual(patchBody(draft), { cri: 81, nominal_input_power_w: 35, luminaire_colour: 'warm white' })
  assert.equal(canWrite(draft), true)
})

test('an untouched field is not sent and an emptied text is sent empty', () => {
  const draft = touched(EMPTY_DRAFT, 'texts', 'oem_name', '')
  assert.deepEqual(patchBody(draft), { oem_name: '' })
  assert.equal(canWrite(draft), true)
})

test('a draft with no touched field cannot be written', () => {
  assert.equal(canWrite(EMPTY_DRAFT), false)
  assert.deepEqual(draftNotes(EMPTY_DRAFT), [])
})

test('a touched number that is not whole, emptied included, blocks Write', () => {
  for (const raw of ['8,5', '']) {
    const draft = touched(EMPTY_DRAFT, 'numbers', 'nominal_min_ac_voltage_v', raw)
    assert.deepEqual(draftNotes(draft), [WHOLE_NUMBER_NOTE], raw)
    assert.equal(canWrite(draft), false, raw)
  }
})

test('one text field per write; a second one blocks Write', () => {
  const one = touched(EMPTY_DRAFT, 'texts', 'luminaire_identification', 'A')
  assert.equal(canWrite(one), true)
  const two = touched(one, 'texts', 'luminaire_colour', 'B')
  assert.deepEqual(draftNotes(two), [ONE_TEXT_NOTE])
  assert.equal(canWrite(two), false)
  assert.equal(canWrite(touched(one, 'numbers', 'cri', '90')), true)
})

test('a succeeded operation closes the editor', () => {
  assert.deepEqual(writeOutcome({ status: 'succeeded' }, NO_START), { kind: 'succeeded' })
})

test('a refused PATCH shows the server message', () => {
  for (const status of [409, 422]) {
    const text = `${status} conflict — bank1_not_read`
    assert.deepEqual(writeOutcome(null, refusedStart(status, text)), { kind: 'refused', text })
  }
})

test('a failed operation names its code in the operation toast wording', () => {
  const verify = writeOutcome({ status: 'failed', error: { code: 'verify_failed' } }, NO_START)
  assert.equal(verify.kind, 'failed')
  assert.match('text' in verify ? verify.text : '', /^The write failed: the gear read back another value/)
  const conflict = writeOutcome(
    { status: 'failed', error: { code: 'conflict', message: 'device_identity_changed' } },
    NO_START,
  )
  assert.deepEqual(conflict, { kind: 'failed', text: 'The write failed: conflict — device_identity_changed.' })
  assert.deepEqual(writeOutcome({ status: 'failed' }, NO_START), { kind: 'failed', text: 'The write failed.' })
})

test('a write that stood down says the gear may hold part of it', () => {
  const out = writeOutcome({ status: 'failed', error: { code: 'preempted' } }, NO_START)
  assert.equal(out.kind, 'failed')
  assert.match('text' in out ? out.text : '', /^Stood down for an operator command; the gear may hold part of the write/)
})

test('every end that is not known says so', () => {
  const notKnown = { kind: 'not_known', text: OUTCOME_NOT_KNOWN }
  for (const status of ['timed_out', 'cancelled', 'unknown', 'running', 'accepted']) {
    assert.deepEqual(writeOutcome({ status }, NO_START), notKnown, status)
  }
  assert.deepEqual(writeOutcome(null, NO_START), notKnown, 'the follow lost the operation')
  assert.deepEqual(writeOutcome(null, refusedStart(undefined, 'Failed to fetch')), notKnown, 'network error')
  assert.deepEqual(writeOutcome(null, refusedStart(504, '504 timeout')), notKnown, 'gateway timeout')
  assert.deepEqual(writeOutcome(null, refusedStart(502, '502 http_502')), notKnown, 'bad gateway')
})

test('a refusal after an accepted attempt of the same Write is not known', () => {
  const starts: WriteStarts = { accepted: true, refusal: { status: 409, text: '409 conflict' } }
  assert.deepEqual(writeOutcome(null, starts), { kind: 'not_known', text: OUTCOME_NOT_KNOWN })
})
