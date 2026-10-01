import assert from 'node:assert/strict'
import test from 'node:test'

import { cctSamples, curveLines, levelSamples } from '../src/screens/hcl-curve.js'

const at = (offset_minutes: number, value: number | null) => ({ offset_minutes, value })

test('consecutive values form one ramp under interpolation', () => {
  assert.deepEqual(curveLines([at(360, 80), at(480, 200), at(1080, 40)], false), [
    [[360, 80], [480, 200], [1080, 40], [1440, 40]],
  ])
})

test('a point without a value holds the line flat up to it and breaks it there', () => {
  const lines = curveLines(
    [at(360, 80), at(480, null), at(600, 200), at(720, 120), at(900, null), at(1080, 40)],
    false,
  )
  assert.deepEqual(lines, [
    [[360, 80], [480, 80]],
    [[600, 200], [720, 120], [900, 120]],
    [[1080, 40], [1440, 40]],
  ])
})

test('a stepped schedule holds each value until the next point', () => {
  assert.deepEqual(curveLines([at(360, 80), at(480, 200), at(600, null)], true), [
    [[360, 80], [480, 80], [480, 200], [600, 200]],
  ])
})

test('a curve with no value has no line', () => {
  assert.deepEqual(curveLines([at(360, null), at(480, null)], false), [])
})

test('the last value is held until midnight, as the scheduler sends it', () => {
  assert.deepEqual(curveLines([at(360, 80), at(1080, 40)], true), [
    [[360, 80], [1080, 80], [1080, 40], [1440, 40]],
  ])
})

test('a single point draws a held line, not just a dot', () => {
  assert.deepEqual(curveLines([at(600, 120)], false), [[[600, 120], [1440, 120]]])
})

const point = (level_mode: 'absolute' | 'last_active' | 'none', level: number | null) => ({
  time_ref: 'absolute' as const,
  offset_minutes: 360,
  level_mode,
  level,
  color_temperature_kelvin: 3000,
})

test('only an absolute level is a value; last_active and none break the level line', () => {
  assert.deepEqual(
    levelSamples([point('absolute', 80), point('last_active', null), point('none', null)]).map(
      (s) => s.value,
    ),
    [80, null, null],
  )
  assert.deepEqual(cctSamples([point('none', null)]).map((s) => s.value), [3000])
})
