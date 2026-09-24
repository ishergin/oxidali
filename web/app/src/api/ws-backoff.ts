export const BACKOFF_MS = [1000, 2000, 4000, 8000, 15000] as const

export const BUSY_RETRY_MS = 60_000

export const CLOSE_TRY_AGAIN_LATER = 1013

export function nextReconnectDelay(attempt: number, refusedForCapacity: boolean): number {
  const rung = BACKOFF_MS[Math.min(Math.max(attempt, 0), BACKOFF_MS.length - 1)]
  return refusedForCapacity ? Math.max(rung, BUSY_RETRY_MS) : rung
}
