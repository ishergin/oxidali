import assert from 'node:assert/strict'
import test from 'node:test'

import type { Operation, OperationAccepted } from '../src/api/types.js'
import { runWithRetries, type RetryPorts } from '../src/op-retry.js'

const ACCEPTED: OperationAccepted = { operation_id: 'op', type: 'attribute_write', status: 'accepted' }
const PREEMPTED: Operation = { operation_id: 'op', type: 'attribute_write', status: 'failed', error: { code: 'preempted' } }
const SUCCEEDED: Operation = { operation_id: 'op', type: 'attribute_write', status: 'succeeded' }

interface Seen {
  starts: number
  willRetry: boolean[]
  retried: boolean[]
  pauses: number
}

function ports(starts: ('accept' | 'refuse')[], ends: Operation[]): { ports: RetryPorts; seen: Seen } {
  const seen: Seen = { starts: 0, willRetry: [], retried: [], pauses: 0 }
  return {
    seen,
    ports: {
      start: async () => {
        if (starts[seen.starts++] === 'refuse') throw new Error('409 conflict')
        return ACCEPTED
      },
      track: async (_accepted, willRetry) => {
        seen.willRetry.push(willRetry)
        return ends[seen.willRetry.length - 1] ?? null
      },
      startFailed: (_e, retried) => seen.retried.push(retried),
      pause: async () => {
        seen.pauses++
      },
    },
  }
}

test('a first start that fails is not a retry', async () => {
  const { ports: p, seen } = ports(['refuse'], [])
  assert.equal(await runWithRetries(p, 3), null)
  assert.deepEqual(seen.retried, [false])
  assert.deepEqual(seen.willRetry, [])
})

test('a start that fails after an accepted attempt stood down is a retry', async () => {
  const { ports: p, seen } = ports(['accept', 'refuse'], [PREEMPTED])
  assert.equal(await runWithRetries(p, 3), null)
  assert.deepEqual(seen.retried, [true])
  assert.equal(seen.pauses, 1)
})

test('a run that keeps standing down is sent again up to the budget, and says when it will not be', async () => {
  const { ports: p, seen } = ports([], [PREEMPTED, PREEMPTED, PREEMPTED, PREEMPTED, PREEMPTED])
  assert.equal(await runWithRetries(p, 3), PREEMPTED)
  assert.equal(seen.starts, 4)
  assert.deepEqual(seen.willRetry, [true, true, true, false])
  assert.equal(seen.pauses, 3)
})

test('any end other than a stand-down is returned at once', async () => {
  const { ports: p, seen } = ports([], [SUCCEEDED])
  assert.equal(await runWithRetries(p, 3), SUCCEEDED)
  assert.equal(seen.starts, 1)
  assert.equal(seen.pauses, 0)
})
