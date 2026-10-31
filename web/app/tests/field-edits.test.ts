import assert from 'node:assert/strict'
import test from 'node:test'

import {
  fieldPatch,
  fieldsOnScreen,
  sendFieldsOnFreshRead,
  withFieldEdit,
} from '../src/screens/field-edits.js'

const stored = {
  dt8_auto_activation_repair: true,
  application_active: true,
  device_short_address: 5 as number | null,
}

test('an untouched record sends nothing', () => {
  assert.deepEqual(fieldPatch({}, stored), {})
})

test('a field the wire changed under an open draft is shown as it is now and never sent', () => {
  const edits = withFieldEdit({}, { dt8_auto_activation_repair: false }, stored)
  assert.ok(edits)
  const stoodDown = { ...stored, application_active: false, device_short_address: 9 }
  assert.equal(fieldsOnScreen(edits, stoodDown).application_active, false)
  assert.equal(fieldsOnScreen(edits, stoodDown).device_short_address, 9)
  assert.deepEqual(fieldPatch(edits, stoodDown), { dt8_auto_activation_repair: false })
})

test('an edit back to the value shown keeps no draft, so the screen follows the controller', () => {
  const edits = withFieldEdit({}, { application_active: false }, stored)
  assert.ok(edits)
  assert.equal(withFieldEdit(edits, { application_active: true }, stored), null)
})

test('a draft the controller has caught up with is dropped', () => {
  const edits = withFieldEdit({}, { device_short_address: null }, stored)
  assert.ok(edits)
  assert.equal(withFieldEdit(edits, {}, { ...stored, device_short_address: null }), null)
})

test('the patch is taken against a fresh read, and nothing is sent when the controller already holds it', async () => {
  const edits = { device_short_address: 7, application_active: false }
  const sent: object[] = []
  const send = async (patch: object) => {
    sent.push(patch)
    return 'sent'
  }
  const fresh = async () => ({ ...stored, application_active: false })
  assert.equal(await sendFieldsOnFreshRead(edits, fresh, send), 'sent')
  assert.deepEqual(sent, [{ device_short_address: 7 }])
  const caughtUp = async () => ({ ...stored, application_active: false, device_short_address: 7 })
  assert.equal(await sendFieldsOnFreshRead(edits, caughtUp, send), null)
  assert.equal(sent.length, 1)
})
