import assert from 'node:assert/strict'
import test from 'node:test'

import type { Operation, OperationAccepted } from '../src/api/types.js'
import { opFailure } from '../src/format.js'
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
  writeBank1,
  writeOutcome,
  type WriteOutcome,
  type WriteStarts,
  type WriteSteps,
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

test('a failed operation shows the toast detail word for word', () => {
  const ops: Pick<Operation, 'status' | 'error'>[] = [
    { status: 'failed', error: { code: 'conflict', message: 'device_identity_changed' } },
    { status: 'failed', error: { code: 'verify_failed', message: 'cri' } },
    { status: 'failed', error: { code: 'verify_unanswered', message: 'bank1_readback' } },
    { status: 'failed', error: { code: 'verify_failed', message: 'bank1_lock_not_restored' } },
  ]
  for (const op of ops) {
    assert.deepEqual(writeOutcome(op, NO_START), { kind: 'failed', text: opFailure(op) }, op.error?.message)
  }
  assert.deepEqual(writeOutcome({ status: 'failed' }, NO_START), { kind: 'failed', text: 'The write failed.' })
})

test('a failed line never calls an unproved write a failed one', () => {
  const op = { status: 'failed', error: { code: 'verify_unanswered', message: 'bank1_readback' } }
  const out = writeOutcome(op, NO_START)
  assert.equal(out.kind, 'failed')
  assert.doesNotMatch('text' in out ? out.text : '', /write failed|did not take/)
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

const ACCEPTED: OperationAccepted = { operation_id: 'pd-attr-0-9-1', type: 'attribute_write', status: 'accepted' }

interface Recorded {
  events: string[]
  outcome: WriteOutcome | null
}

type Run = WriteSteps['run']

function fakeSteps(run: Run, reload: () => Promise<void> = async () => {}): { steps: WriteSteps; seen: Recorded } {
  const seen: Recorded = { events: [], outcome: null }
  const steps: WriteSteps = {
    run,
    patch: async () => ACCEPTED,
    describe: (e) => ({ status: (e as { status?: number }).status, text: String((e as Error).message) }),
    setBusy: (busy) => seen.events.push(busy ? 'lock' : 'unlock'),
    setOutcome: (outcome) => {
      seen.outcome = outcome
      if (outcome) seen.events.push(`line:${outcome.kind}`)
    },
    reload: async () => {
      seen.events.push('reload')
      await reload()
      seen.events.push('reloaded')
    },
    onClose: () => seen.events.push('close'),
  }
  return { steps, seen }
}

const DRAFT = touched(EMPTY_DRAFT, 'numbers', 'cri', '90')

const ending = (op: Pick<Operation, 'status' | 'error'> | null): Run => async (start) => {
  await start()
  return op
}

const ENDINGS: [string, Run][] = [
  ['succeeded', ending({ status: 'succeeded' })],
  ['failed', ending({ status: 'failed', error: { code: 'verify_failed', message: 'cri' } })],
  ['lost', ending(null)],
  ['timed out', ending({ status: 'timed_out' })],
  ['thrown', async () => Promise.reject(new Error('boom'))],
]

test('every ending unlocks the form and reads the device again, once', async () => {
  for (const [name, run] of ENDINGS) {
    const { steps, seen } = fakeSteps(run)
    await writeBank1(DRAFT, steps)
    const events = seen.events.filter((e) => !e.startsWith('line:') && e !== 'close')
    assert.deepEqual(events, ['lock', 'unlock', 'reload', 'reloaded'], name)
  }
})

test('the editor closes only on success, and every other ending keeps a line', async () => {
  for (const [name, run] of ENDINGS) {
    const { steps, seen } = fakeSteps(run)
    await writeBank1(DRAFT, steps)
    const closed = seen.events.includes('close')
    assert.equal(closed, name === 'succeeded', name)
    assert.equal(seen.outcome === null, name === 'succeeded', name)
  }
})

test('a run that throws ends as not known', async () => {
  const { steps, seen } = fakeSteps(async () => Promise.reject(new Error('boom')))
  await writeBank1(DRAFT, steps)
  assert.deepEqual(seen.outcome, { kind: 'not_known', text: OUTCOME_NOT_KNOWN })
})

test('the editor closes on the fresh values, after the reload has landed', async () => {
  let land = () => {}
  const landed = new Promise<void>((resolve) => {
    land = resolve
  })
  const { steps, seen } = fakeSteps(ending({ status: 'succeeded' }), () => landed)
  const done = writeBank1(DRAFT, steps)
  await new Promise((resolve) => setImmediate(resolve))
  assert.deepEqual(seen.events, ['lock', 'unlock', 'reload'])
  land()
  await done
  assert.deepEqual(seen.events, ['lock', 'unlock', 'reload', 'reloaded', 'close'])
})

const startingTimes = (attempts: number): Run => async (start) => {
  for (let attempt = 1; attempt < attempts; attempt++) await start()
  try {
    await start()
    return { status: 'succeeded' }
  } catch {
    return null
  }
}

function refusedPatch(steps: WriteSteps, refusals: number[]): WriteSteps {
  let call = 0
  const patch = async () => {
    const status = refusals[call++]
    if (status) throw Object.assign(new Error(`${status} conflict — bank1_not_read`), { status })
    return ACCEPTED
  }
  return { ...steps, patch }
}

test('a 4xx to the first PATCH is a refusal; after an accepted attempt it is not known', async () => {
  const first = fakeSteps(startingTimes(1))
  await writeBank1(DRAFT, refusedPatch(first.steps, [409]))
  assert.deepEqual(first.seen.outcome, { kind: 'refused', text: '409 conflict — bank1_not_read' })
  const retry = fakeSteps(startingTimes(2))
  await writeBank1(DRAFT, refusedPatch(retry.steps, [0, 409]))
  assert.deepEqual(retry.seen.outcome, { kind: 'not_known', text: OUTCOME_NOT_KNOWN })
})
