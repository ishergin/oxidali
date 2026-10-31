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

export function withoutSent<T extends object>(edits: Partial<T>, sent: Partial<T>): Partial<T> | null {
  const kept: Partial<T> = {}
  for (const key of Object.keys(edits) as (keyof T)[]) {
    if (sent[key] !== edits[key]) kept[key] = edits[key]
  }
  return Object.keys(kept).length > 0 ? kept : null
}

export async function sendFieldsOnFreshRead<T extends object>(
  edits: Partial<T>,
  read: () => Promise<T>,
  send: (patch: Partial<T>) => Promise<unknown>,
): Promise<Partial<T>> {
  const patch = fieldPatch(edits, await read())
  if (Object.keys(patch).length > 0) await send(patch)
  return patch
}
