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

export async function settle<T>(request: () => Promise<T>): Promise<T | null> {
  try {
    return await request()
  } catch {
    return null
  }
}
