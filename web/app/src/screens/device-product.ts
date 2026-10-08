export interface GtinSummary {
  short_address: number
  gtin?: number | null
}

export function productGtin(
  summaries: readonly GtinSummary[] | undefined,
  short: number,
): number | null {
  return summaries?.find((device) => device.short_address === short)?.gtin ?? null
}
