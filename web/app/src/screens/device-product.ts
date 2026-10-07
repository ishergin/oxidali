export interface GtinSummary {
  short_address: number
  gtin?: number | null
}

export function productGtin(
  sectionGtin: number | null,
  summaries: readonly GtinSummary[] | undefined,
  short: number,
): number | null {
  if (sectionGtin != null) return sectionGtin
  return summaries?.find((device) => device.short_address === short)?.gtin ?? null
}
