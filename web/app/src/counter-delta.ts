const U32_MODULUS = 2 ** 32

export function deltaOf(now: number, before: number): number {
  return now >= before ? now - before : now + U32_MODULUS - before
}

export type Flat = Record<string, number>

export type NamedCount = { name: string; count: number }

export function flatten(value: unknown, prefix: string, out: Flat): Flat {
  if (typeof value === 'number') {
    out[prefix] = value
    return out
  }
  if (Array.isArray(value)) {
    value.forEach((v, i) => flatten(v, `${prefix}[${i}]`, out))
    return out
  }
  if (value && typeof value === 'object') {
    for (const [k, v] of Object.entries(value)) {
      flatten(v, prefix ? `${prefix}.${k}` : k, out)
    }
  }
  return out
}

export function countsByName(counts: readonly NamedCount[]): Record<string, number> {
  return Object.fromEntries(counts.map((entry) => [entry.name, entry.count]))
}
