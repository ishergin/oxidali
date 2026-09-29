export const RULE_SETTLE_MS = 1000
export const FEED_CAPACITY = 100

export interface InputFeedPayload {
  scheme?: number | null
  short_address?: number | null
  instance_number?: number | null
  event?: string | null
  event_info?: number | null
}

export interface RuleActivationPayload {
  rule_name?: string | null
  dry?: boolean | null
  effects?: number | null
  partial?: number | null
  trigger_to_publish_ms?: number | null
}

export interface FeedRow {
  seq: number
  at: string
  short: number | null
  instance: number | null
  event: string | null
  scheme: number | null
  info: number | null
  lifecycle: boolean
  atMs: number
  rule?: string | null
  partial?: number
  ms?: number | null
}

export interface FeedStamp {
  seq: number
  at: string
  atMs: number
}

export type FeedVerdict = 'unattributable' | 'pending' | 'fired' | 'unmatched'

export function feedRow(payload: InputFeedPayload, lifecycle: boolean, stamp: FeedStamp): FeedRow {
  return {
    seq: stamp.seq,
    at: stamp.at,
    short: payload.short_address ?? null,
    instance: payload.instance_number ?? null,
    event: payload.event ?? null,
    scheme: payload.scheme ?? null,
    info: payload.event_info ?? null,
    lifecycle,
    atMs: stamp.atMs,
  }
}

export function appendRow(rows: readonly FeedRow[], row: FeedRow): FeedRow[] {
  const next = [...rows, row]
  return next.length > FEED_CAPACITY ? next.slice(next.length - FEED_CAPACITY) : next
}

const awaitsActivation = (row: FeedRow, now: number) =>
  row.rule === undefined && row.short !== null && now - row.atMs < RULE_SETTLE_MS

export function attachActivation(
  rows: FeedRow[],
  activation: RuleActivationPayload,
  now: number,
): FeedRow[] {
  if (activation.dry) return rows
  const i = rows.findLastIndex((row) => awaitsActivation(row, now))
  if (i < 0) return rows
  const next = rows.slice()
  next[i] = {
    ...rows[i],
    rule: activation.rule_name ?? null,
    partial: activation.partial ?? 0,
    ms: activation.trigger_to_publish_ms ?? null,
  }
  return next
}

export function rowVerdict(row: FeedRow, now: number): FeedVerdict {
  if (row.short === null) return 'unattributable'
  if (row.rule === undefined && now - row.atMs < RULE_SETTLE_MS) return 'pending'
  return row.rule ? 'fired' : 'unmatched'
}

export function nextSettleAt(rows: readonly FeedRow[], now: number): number | null {
  let soonest: number | null = null
  for (const row of rows) {
    if (rowVerdict(row, now) !== 'pending') continue
    const due = row.atMs + RULE_SETTLE_MS
    soonest = soonest === null ? due : Math.min(soonest, due)
  }
  return soonest
}
