export function withGroupToggled<G extends string>(
  selected: readonly G[],
  group: G,
  shown: readonly G[],
): G[] {
  if (selected.includes(group)) return selected.filter((g) => g !== group)
  const unshown = selected.filter((g) => !shown.includes(g))
  return [...shown.filter((g) => g === group || selected.includes(g)), ...unshown]
}
