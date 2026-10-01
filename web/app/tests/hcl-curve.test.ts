import assert from 'node:assert/strict'
import test from 'node:test'

import { levelRuns } from '../src/screens/hcl-curve.js'

const point = (offset_minutes: number, level_mode: string, level: number | null) => ({
  offset_minutes,
  level_mode,
  level,
})

test('consecutive absolute levels form one line', () => {
  const runs = levelRuns([point(360, 'absolute', 80), point(480, 'absolute', 200), point(1080, 'absolute', 40)])
  assert.deepEqual(runs.map((run) => run.map((p) => p.offset_minutes)), [[360, 480, 1080]])
})

test('a last-active or undriven point breaks the level line instead of being bridged', () => {
  const runs = levelRuns([
    point(360, 'absolute', 80),
    point(480, 'last_active', null),
    point(600, 'absolute', 200),
    point(720, 'absolute', 120),
    point(900, 'none', null),
    point(1080, 'absolute', 40),
  ])
  assert.deepEqual(runs.map((run) => run.map((p) => p.offset_minutes)), [[360], [600, 720], [1080]])
})

test('a curve with no absolute level has no line', () => {
  assert.deepEqual(levelRuns([point(360, 'last_active', null), point(480, 'none', null)]), [])
})
