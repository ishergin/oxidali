export type LatestRequestGate = {
  begin: () => number
  invalidate: () => void
  isCurrent: (generation: number) => boolean
}

export function createLatestRequestGate(): LatestRequestGate {
  let generation = 0
  return {
    begin: () => ++generation,
    invalidate: () => {
      generation += 1
    },
    isCurrent: (candidate) => candidate === generation,
  }
}

export async function runLatestRequest<T>(
  gate: LatestRequestGate,
  request: () => Promise<T>,
  accept: (value: T) => void,
  reject: (error: unknown) => void,
): Promise<void> {
  const generation = gate.begin()
  try {
    const value = await request()
    if (gate.isCurrent(generation)) accept(value)
  } catch (error) {
    if (gate.isCurrent(generation)) reject(error)
  }
}
