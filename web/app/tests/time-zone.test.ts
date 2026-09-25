import assert from 'node:assert/strict'
import test from 'node:test'

import {
  civilTime,
  clockSkewLabel,
  posixTzFromBrowser,
  utcOffsetLabel,
} from '../src/time-zone.js'

const NOW = Date.UTC(2026, 8, 25, 9, 4, 31)

const between = (fromMs: number, toMs: number, inside: number, outside: number) =>
  (ms: number) => (ms >= fromMs && ms < toMs ? inside : outside)

const moscow = () => 180

const berlin = between(Date.UTC(2026, 2, 29, 1), Date.UTC(2026, 9, 25, 1), 120, 60)

const newYork = between(Date.UTC(2026, 2, 8, 7), Date.UTC(2026, 10, 1, 6), -240, -300)

const sydney = between(Date.UTC(2026, 3, 4, 16), Date.UTC(2026, 9, 3, 16), 600, 660)

const lordHowe = between(Date.UTC(2026, 3, 4, 15), Date.UTC(2026, 9, 3, 15, 30), 630, 660)

test('a zone without daylight saving is its standard offset alone', () => {
  assert.equal(posixTzFromBrowser(moscow, NOW), '<+03>-3')
  assert.equal(posixTzFromBrowser(() => 0, NOW), '<+00>0')
  assert.equal(posixTzFromBrowser(() => 330, NOW), '<+0530>-5:30')
})

test('Europe/Berlin changes on the last Sunday of March and October', () => {
  assert.equal(posixTzFromBrowser(berlin, NOW), '<+01>-1<+02>,M3.5.0/2,M10.5.0/3')
})

test('America/New_York is west of UTC, so its POSIX offset is positive', () => {
  assert.equal(posixTzFromBrowser(newYork, NOW), '<-05>5<-04>,M3.2.0/2,M11.1.0/2')
})

test('Australia/Sydney starts daylight saving in October and ends it in April', () => {
  assert.equal(posixTzFromBrowser(sydney, NOW), '<+10>-10<+11>,M10.1.0/2,M4.1.0/3')
})

test('a daylight shift other than one hour is written out', () => {
  assert.equal(
    posixTzFromBrowser(lordHowe, NOW),
    '<+1030>-10:30<+11>-11,M10.1.0/2,M4.1.0/2',
  )
})

test('an offset change the POSIX rule cannot express falls back to the offset now', () => {
  const abolished = (ms: number) => (ms < Date.UTC(2026, 4, 1) ? 120 : 180)
  assert.equal(posixTzFromBrowser(abolished, NOW), '<+03>-3')
})

test('the controller time renders in its own zone', () => {
  assert.equal(civilTime(NOW, 180), '2026-09-25 12:04:31')
  assert.equal(civilTime(NOW, -300), '2026-09-25 04:04:31')
  assert.equal(utcOffsetLabel(180), 'UTC+03:00')
  assert.equal(utcOffsetLabel(-210), 'UTC-03:30')
  assert.equal(utcOffsetLabel(0), 'UTC+00:00')
})

test('the skew between controller and browser is named with its direction', () => {
  assert.equal(clockSkewLabel(NOW + 1500, NOW), 'in step')
  assert.equal(clockSkewLabel(NOW - 3000, NOW), 'controller 3 s behind')
  assert.equal(clockSkewLabel(NOW + 600_000, NOW), 'controller 10 min ahead')
  assert.equal(clockSkewLabel(NOW - 3 * 3600_000, NOW), 'controller 3 h behind')
})
