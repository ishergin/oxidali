export function freshSource<T>(
  load: (previous: T | null) => Promise<T>,
  now: () => number,
  freshMs: number,
): () => Promise<T> {
  let value: T | null = null
  let requestedAt: number | null = null
  let last: Promise<T> | null = null
  return () => {
    const at = now()
    if (last !== null && requestedAt !== null && at - requestedAt < freshMs) return last
    requestedAt = at
    last = load(value).then((loaded) => {
      value = loaded
      return loaded
    })
    return last
  }
}

export function settledValue<T>(result: PromiseSettledResult<T>): T | null {
  return result.status === 'fulfilled' ? result.value : null
}
