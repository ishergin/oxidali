export function fieldsOnScreen<T extends object>(edits: Partial<T>, latest: T): T {
  return { ...latest, ...edits }
}

export function fieldPatch<T extends object>(edits: Partial<T>, latest: T): Partial<T> {
  const patch: Partial<T> = {}
  for (const key of Object.keys(edits) as (keyof T)[]) {
    if (edits[key] !== latest[key]) patch[key] = edits[key]
  }
  return patch
}

export function withFieldEdit<T extends object>(
  edits: Partial<T>,
  change: Partial<T>,
  latest: T,
): Partial<T> | null {
  const kept = fieldPatch({ ...edits, ...change }, latest)
  return Object.keys(kept).length > 0 ? kept : null
}

export async function sendFieldsOnFreshRead<T extends object, R>(
  edits: Partial<T>,
  read: () => Promise<T>,
  send: (patch: Partial<T>) => Promise<R>,
): Promise<R | null> {
  const patch = fieldPatch(edits, await read())
  return Object.keys(patch).length > 0 ? send(patch) : null
}
