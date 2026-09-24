import assert from 'node:assert/strict'
import test from 'node:test'

import { createLatestRequestGate, runLatestRequest } from '../src/latest-request.js'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

test('an older response cannot overwrite the newest request', async () => {
  const gate = createLatestRequestGate()
  const first = deferred<string>()
  const second = deferred<string>()
  const accepted: string[] = []
  const failures: unknown[] = []

  const firstRun = runLatestRequest(gate, () => first.promise, accepted.push.bind(accepted), failures.push.bind(failures))
  const secondRun = runLatestRequest(gate, () => second.promise, accepted.push.bind(accepted), failures.push.bind(failures))
  second.resolve('new')
  await secondRun
  first.resolve('old')
  await firstRun

  assert.deepEqual(accepted, ['new'])
  assert.deepEqual(failures, [])
})

test('invalidating a request suppresses its completion', async () => {
  const gate = createLatestRequestGate()
  const pending = deferred<string>()
  const accepted: string[] = []
  const failures: unknown[] = []
  const run = runLatestRequest(gate, () => pending.promise, accepted.push.bind(accepted), failures.push.bind(failures))

  gate.invalidate()
  pending.resolve('stale')
  await run

  assert.deepEqual(accepted, [])
  assert.deepEqual(failures, [])
})
