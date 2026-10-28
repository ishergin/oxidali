export interface PolicyExceptionEntry {
  adapter_id: number
  short_address: number
}

export interface PolicyFields {
  system_failure_level: number | null
  power_on_level: number | null
  apply_on_discovery: boolean
  exceptions: PolicyExceptionEntry[]
}

export const exceptionKey = (e: PolicyExceptionEntry) => `${e.adapter_id}:${e.short_address}`

export const sameExceptions = (a: PolicyExceptionEntry[], b: PolicyExceptionEntry[]) =>
  a.map(exceptionKey).join() === b.map(exceptionKey).join()

export function sortedExceptions(list: PolicyExceptionEntry[]): PolicyExceptionEntry[] {
  return [...list].sort((x, y) => x.adapter_id - y.adapter_id || x.short_address - y.short_address)
}

function rebasedExceptions(base: PolicyFields, draft: PolicyFields, latest: PolicyFields) {
  const before = new Set(base.exceptions.map(exceptionKey))
  const after = new Set(draft.exceptions.map(exceptionKey))
  const kept = latest.exceptions.filter((e) => !before.has(exceptionKey(e)) || after.has(exceptionKey(e)))
  const held = new Set(kept.map(exceptionKey))
  const added = draft.exceptions.filter((e) => !before.has(exceptionKey(e)) && !held.has(exceptionKey(e)))
  return sortedExceptions([...kept, ...added])
}

export function policyPatch(
  base: PolicyFields,
  draft: PolicyFields,
  latest: PolicyFields,
): Partial<PolicyFields> {
  const patch: Partial<PolicyFields> = {}
  if (draft.system_failure_level !== base.system_failure_level) {
    patch.system_failure_level = draft.system_failure_level
  }
  if (draft.power_on_level !== base.power_on_level) patch.power_on_level = draft.power_on_level
  if (draft.apply_on_discovery !== base.apply_on_discovery) {
    patch.apply_on_discovery = draft.apply_on_discovery
  }
  if (!sameExceptions(draft.exceptions, base.exceptions)) {
    patch.exceptions = rebasedExceptions(base, draft, latest)
  }
  return patch
}
