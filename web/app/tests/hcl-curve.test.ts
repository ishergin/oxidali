import assert from 'node:assert/strict'
import test from 'node:test'

import { curveLines } from '../src/screens/hcl-curve.js'

const at = (offset_minutes: number, value: number | null) => ({ offset_minutes, value })

test('consecutive values form one ramp under interpolation', () => {
  assert.deepEqual(curveLines([at(360, 80), at(480, 200), at(1080, 40)], false), [
    [[360, 80], [480, 200], [1080, 40]],
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
    [[1080, 40]],
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
