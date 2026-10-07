import assert from 'node:assert/strict'
import test from 'node:test'

import { isFault as isDiagnosticsFault } from '../src/screens/diagnostics-view.js'
import { isFault as isStatsFault } from '../src/screens/stats-view.js'

test('an MQTT publish that failed is a fault on both screens', () => {
  assert.equal(isDiagnosticsFault('publish_failures_total', 1), true)
  assert.equal(isStatsFault('publish_failures_total', 1), true)
  assert.equal(isStatsFault('publish_failures_total', 0), false)
})

test('a Home Assistant command the bus refused is a fault', () => {
  assert.equal(isDiagnosticsFault('commands_ingress_rejected_total', 1), true)
})

test('a dropped log line is narration, not a fault', () => {
  assert.equal(isDiagnosticsFault('logs_dropped_total', 3), false)
})
