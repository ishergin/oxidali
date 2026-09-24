let table: Record<string, string> | null = null
let pending: Promise<void> | null = null

async function load(): Promise<void> {
  try {
    const res = await fetch('/dali-products.json')
    if (!res.ok) {
      table = {}
      return
    }
    const body = (await res.json()) as { products?: Record<string, string> }
    table = body.products ?? {}
  } catch {
    table = {}
  }
}

export function ensureProductsLoaded(): void {
  if (table !== null || pending !== null) return
  pending = load().finally(() => {
    pending = null
  })
}

export function productName(gtin: number | null): string | null {
  if (gtin == null || table === null) return null
  return table[String(gtin)] ?? null
}
