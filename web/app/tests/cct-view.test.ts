import assert from 'node:assert/strict'
import test from 'node:test'

import {
  CCT_MAX_K,
  CCT_MIN_K,
  CCT_UNREPORTED_K,
  cctSliderView,
} from '../src/components/cct-view.js'

const STRIP = { min_kelvin: 2500, max_kelvin: 6250 }

test('a colour below the fallback minimum is shown, not replaced', () => {
  const view = cctSliderView(2500, null, STRIP)
  assert.equal(view.value, 2500)
  assert.equal(view.clamped, null)
  assert.deepEqual([view.min, view.max], [2500, 6250])
})

test('the fallback bounds apply only while the fixture has reported none', () => {
  const view = cctSliderView(2500, null, null)
  assert.deepEqual([view.min, view.max], [CCT_MIN_K, CCT_MAX_K])
  assert.equal(view.value, CCT_MIN_K)
  assert.equal(view.clamped, 2500)
})

test('a colour the fixture cannot reach is clamped and named', () => {
  const view = cctSliderView(6500, null, STRIP)
  assert.equal(view.value, 6250)
  assert.equal(view.clamped, 6500)
})

test('no observation rests the thumb on the placeholder, inside the range', () => {
  assert.equal(cctSliderView(null, null, STRIP).value, CCT_UNREPORTED_K)
  assert.equal(cctSliderView(undefined, null, STRIP).clamped, null)
  const narrow = cctSliderView(null, null, { min_kelvin: 2500, max_kelvin: 3000 })
  assert.equal(narrow.value, 3000)
  assert.equal(narrow.clamped, null)
})

test('a reading outside the product window is not a colour', () => {
  for (const bogus of [0, 2, 999, 65535]) {
    const view = cctSliderView(bogus, null, STRIP)
    assert.equal(view.value, CCT_UNREPORTED_K, `${bogus} K must not reach the thumb`)
    assert.equal(view.clamped, null)
  }
})

test('a drag wins over the observation and never reads as clamped', () => {
  const view = cctSliderView(6500, 2500, STRIP)
  assert.equal(view.value, 2500)
  assert.equal(view.clamped, null)
})
