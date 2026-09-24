import assert from 'node:assert/strict'
import test from 'node:test'

import { lampState } from '../src/format.js'
import type { RuntimeState } from '../src/api/types.js'

function state(over: Partial<RuntimeState> = {}): RuntimeState {
  return {
    power: 'on',
    level: 180,
    color_mode: 'brightness',
    color_temperature_kelvin: null,
    xy: null,
    rgb: null,
    status: null,
    failure_status: null,
    value_source: 'api',
    last_seen_ms: null,
    last_dapc_source: null,
    error: null,
    ...over,
  } as RuntimeState
}

const POLLING = true
const NOT_POLLING = false

test('a poller readback is confirmed, whatever else is true', () => {
  const s = lampState(state({ value_source: 'poller' }), NOT_POLLING)
  assert.equal(s.cls, 'on')
  assert.ok(s.glow, 'a confirmed bright level keeps its glow')
})

test('a commanded value with nothing polling is ringed', () => {
  const s = lampState(state({ value_source: 'api' }), NOT_POLLING)
  assert.equal(s.cls, 'on unconfirmed')
  assert.equal(s.glow, undefined, 'the glow claims brightness nothing verified')
  assert.match(s.title, /polling is off/)
})

test('a commanded value is NOT ringed while the poller is running', () => {
  const s = lampState(state({ value_source: 'api' }), POLLING)
  assert.equal(s.cls, 'on')
})

test('a stored status byte does not stand in for a read', () => {
  const read = { raw: 0, lamp_failure: false, gear_failure: false } as unknown
  const withStatus = lampState(
    state({ value_source: 'api', status: read as RuntimeState['status'] }),
    NOT_POLLING,
  )
  assert.equal(withStatus.cls, 'on unconfirmed', 'a stale status byte confirms nothing')

  const withoutStatus = lampState(state({ value_source: 'api' }), NOT_POLLING)
  assert.equal(withStatus.cls, withoutStatus.cls, 'status must not change the verdict')
})

test('a sniffer commit is an echo, not an observation', () => {
  const s = lampState(state({ value_source: 'sniffer' }), NOT_POLLING)
  assert.equal(s.cls, 'on unconfirmed')
})

test('absence outranks the ring, and keeps the level as a qualifier', () => {
  const s = lampState(
    state({ value_source: 'api', error: { code: 'device_absent' } }),
    NOT_POLLING,
  )
  assert.equal(s.cls, 'absent')
  assert.equal(s.qualifier, 'was 180', 'what was asked for survives as evidence')
})

test('off and never-observed are their own states, not rings', () => {
  assert.equal(lampState(state({ power: 'off' }), NOT_POLLING).cls, 'off')
  assert.equal(lampState(state({ power: 'unknown' }), NOT_POLLING).cls, 'unknown')
})
