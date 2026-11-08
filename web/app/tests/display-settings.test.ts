import assert from 'node:assert/strict'
import test from 'node:test'

import type { DisplaySettings } from '../src/api/types.js'
import {
  displayPatch,
  screensaverSeconds,
  withPageToggled,
} from '../src/screens/display-settings.js'

const stored: DisplaySettings = {
  enabled: true,
  i2c_address: 0x3c,
  rotation_deg: 0,
  screensaver_timeout_s: 0,
  pages: ['summary'],
}

test('a typed screensaver time lands in the domain the server accepts', () => {
  assert.equal(screensaverSeconds('-5', 300), 0)
  assert.equal(screensaverSeconds('0', 300), 0)
  assert.equal(screensaverSeconds('1', 300), 10)
  assert.equal(screensaverSeconds('9', 300), 10)
  assert.equal(screensaverSeconds('10', 300), 10)
  assert.equal(screensaverSeconds('86400', 300), 86_400)
  assert.equal(screensaverSeconds('86401', 300), 86_400)
})

test('a screensaver time that is not a number keeps the one shown', () => {
  assert.equal(screensaverSeconds('abc', 300), 300)
  assert.equal(screensaverSeconds('', 60), 60)
})

test('switching a page on places it in the order the panel shows them', () => {
  assert.deepEqual(withPageToggled(['bus'], 'summary'), ['summary', 'bus'])
  assert.deepEqual(withPageToggled(['summary', 'bus'], 'network'), ['summary', 'network', 'bus'])
  assert.deepEqual(withPageToggled(['activity', 'summary'], 'bus'), ['summary', 'bus', 'activity'])
})

test('switching the only page off leaves an empty set rather than refusing', () => {
  assert.deepEqual(withPageToggled(['summary'], 'summary'), [])
})

test('an untouched form sends nothing', () => {
  assert.deepEqual(displayPatch({ ...stored, pages: ['summary'] }, stored), {})
})

test('the patch carries only the fields that differ from the controller', () => {
  const next: DisplaySettings = { ...stored, rotation_deg: 180, screensaver_timeout_s: 300 }
  assert.deepEqual(displayPatch(next, stored), { rotation_deg: 180, screensaver_timeout_s: 300 })
})

test('a changed page set goes out whole, in the panel order', () => {
  const next: DisplaySettings = { ...stored, pages: withPageToggled(stored.pages, 'bus') }
  assert.deepEqual(displayPatch(next, stored), { pages: ['summary', 'bus'] })
})

test('turning the panel off is a change even when nothing else moves', () => {
  assert.deepEqual(displayPatch({ ...stored, enabled: false }, stored), { enabled: false })
})
