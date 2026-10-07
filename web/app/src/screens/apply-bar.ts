export interface GearRow {
  virtual_lamp_id: number
  differs: boolean
}

export interface LampBinding {
  virtual_lamp_id: number
  binding?: object | null
}

export type ApplyBar = { kind: 'edits' | 'retry'; count: number } | null

export function boundLamps(lamps: readonly LampBinding[]): Set<number> {
  return new Set(lamps.filter((lamp) => lamp.binding != null).map((lamp) => lamp.virtual_lamp_id))
}

export function applyBar(
  edits: number,
  rows: readonly GearRow[],
  bound: ReadonlySet<number>,
): ApplyBar {
  if (edits > 0) return { kind: 'edits', count: edits }
  const differing = rows.filter((row) => row.differs && bound.has(row.virtual_lamp_id)).length
  return differing > 0 ? { kind: 'retry', count: differing } : null
}
