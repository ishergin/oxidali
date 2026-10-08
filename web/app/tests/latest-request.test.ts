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

test('a request a newer one superseded settles only once the newest has landed', async () => {
  const gate = createLatestRequestGate()
  const first = deferred<string>()
  const second = deferred<string>()
  const accepted: string[] = []
  const ignore = () => {}

  const firstRun = runLatestRequest(gate, () => first.promise, accepted.push.bind(accepted), ignore)
  const secondRun = runLatestRequest(gate, () => second.promise, accepted.push.bind(accepted), ignore)
  first.resolve('old')
  let firstSettled = false
  void firstRun.then(() => {
    firstSettled = true
  })
  await new Promise((done) => setTimeout(done, 0))
  assert.equal(firstSettled, false, 'the superseded reload resolved before fresh data')

  second.resolve('new')
  await firstRun
  await secondRun
  assert.deepEqual(accepted, ['new'])
})
