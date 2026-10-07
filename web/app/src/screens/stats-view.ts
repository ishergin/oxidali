import { countsByName, type NamedCount } from '../counter-delta.js'

type WithRuleStats = { rules: { stats: readonly NamedCount[] } }

export function deltaView<T extends WithRuleStats>(data: T) {
  return { ...data, rules: { ...data.rules, stats: countsByName(data.rules.stats) } }
}

const FAULT_KEYS = new Set([
  'commands_ingress_overflow_total',
  'errors_total',
  'confirmation_timeouts_total',
  'timed_out_total',
  'events_dropped_total',
  'publish_failures_total',
])

export function isFault(key: string, value: number): boolean {
  return FAULT_KEYS.has(key) && value > 0
}
