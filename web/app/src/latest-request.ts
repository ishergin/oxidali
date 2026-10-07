export type LatestRequestGate = {
  begin: () => number
  invalidate: () => void
  isCurrent: (generation: number) => boolean
  follow: (run: Promise<void>) => void
  newest: () => Promise<void>
}

export function createLatestRequestGate(): LatestRequestGate {
  let generation = 0
  let newest: Promise<void> = Promise.resolve()
  return {
    begin: () => ++generation,
    invalidate: () => {
      generation += 1
    },
    isCurrent: (candidate) => candidate === generation,
    follow: (run) => {
      newest = run
    },
    newest: () => newest,
  }
}

export async function runLatestRequest<T>(
  gate: LatestRequestGate,
  request: () => Promise<T>,
  accept: (value: T) => void,
  reject: (error: unknown) => void,
): Promise<void> {
  const generation = gate.begin()
  const run = (async () => {
    try {
      const value = await request()
      if (gate.isCurrent(generation)) accept(value)
    } catch (error) {
      if (gate.isCurrent(generation)) reject(error)
    }
  })()
  gate.follow(run)
  let waited = run
  await run
  for (let next = gate.newest(); next !== waited; next = gate.newest()) {
    waited = next
    await next
  }
}
