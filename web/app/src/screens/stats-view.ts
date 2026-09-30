import { countsByName, type NamedCount } from '../counter-delta.js'

type WithRuleStats = { rules: { stats: readonly NamedCount[] } }

export function deltaView<T extends WithRuleStats>(data: T) {
  return { ...data, rules: { ...data.rules, stats: countsByName(data.rules.stats) } }
}
