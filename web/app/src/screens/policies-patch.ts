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

export interface ExceptionEdits {
  added: PolicyExceptionEntry[]
  removed: PolicyExceptionEntry[]
}

export type PolicyEdits = Partial<Omit<PolicyFields, 'exceptions'>> & { exceptions?: ExceptionEdits }

export const exceptionKey = (e: PolicyExceptionEntry) => `${e.adapter_id}:${e.short_address}`

export const sameExceptions = (a: PolicyExceptionEntry[], b: PolicyExceptionEntry[]) =>
  a.map(exceptionKey).join() === b.map(exceptionKey).join()

export function sortedExceptions(list: PolicyExceptionEntry[]): PolicyExceptionEntry[] {
  return [...list].sort((x, y) => x.adapter_id - y.adapter_id || x.short_address - y.short_address)
}

const keysOf = (list: PolicyExceptionEntry[]) => new Set(list.map(exceptionKey))

const without = (list: PolicyExceptionEntry[], gone: Set<string>) =>
  list.filter((e) => !gone.has(exceptionKey(e)))

function exceptionsWith(latest: PolicyExceptionEntry[], edits?: ExceptionEdits) {
  if (!edits) return latest
  const kept = without(latest, keysOf(edits.removed))
  return sortedExceptions([...kept, ...without(edits.added, keysOf(kept))])
}

function mergedExceptionEdits(
  shown: PolicyExceptionEntry[],
  prior: ExceptionEdits | undefined,
  wanted: PolicyExceptionEntry[],
): ExceptionEdits {
  const added = without(wanted, keysOf(shown))
  const removed = without(shown, keysOf(wanted))
  return {
    added: [...without(prior?.added ?? [], keysOf(removed)), ...added],
    removed: [...without(prior?.removed ?? [], keysOf(added)), ...removed],
  }
}

export function policyOnScreen<T extends PolicyFields>(edits: PolicyEdits, latest: T): T {
  const { exceptions, ...levels } = edits
  return { ...latest, ...levels, exceptions: exceptionsWith(latest.exceptions, exceptions) }
}

export function policyPatch(edits: PolicyEdits, latest: PolicyFields): Partial<PolicyFields> {
  const patch: Partial<PolicyFields> = {}
  const { system_failure_level, power_on_level, apply_on_discovery } = edits
  if (system_failure_level !== undefined && system_failure_level !== latest.system_failure_level) {
    patch.system_failure_level = system_failure_level
  }
  if (power_on_level !== undefined && power_on_level !== latest.power_on_level) {
    patch.power_on_level = power_on_level
  }
  if (apply_on_discovery !== undefined && apply_on_discovery !== latest.apply_on_discovery) {
    patch.apply_on_discovery = apply_on_discovery
  }
  const list = exceptionsWith(latest.exceptions, edits.exceptions)
  if (!sameExceptions(list, sortedExceptions(latest.exceptions))) patch.exceptions = list
  return patch
}

export function withEdit(
  edits: PolicyEdits,
  change: Partial<PolicyFields>,
  latest: PolicyFields,
): PolicyEdits | null {
  const { exceptions: wanted, ...levels } = change
  const next: PolicyEdits = { ...edits, ...levels }
  if (wanted) {
    const shown = policyOnScreen(edits, latest).exceptions
    next.exceptions = mergedExceptionEdits(shown, edits.exceptions, wanted)
  }
  const { exceptions, ...changed } = policyPatch(next, latest)
  const kept: PolicyEdits = exceptions ? { ...changed, exceptions: next.exceptions } : changed
  return Object.keys(kept).length > 0 ? kept : null
}

export async function sendOnFreshRead<R>(
  edits: PolicyEdits,
  read: () => Promise<PolicyFields>,
  send: (patch: Partial<PolicyFields>) => Promise<R>,
): Promise<R> {
  return send(policyPatch(edits, await read()))
}
