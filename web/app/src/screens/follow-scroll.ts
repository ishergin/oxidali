export const STICK_EPSILON_PX = 24

export function isAtBottom(scrollHeight: number, scrollTop: number, clientHeight: number): boolean {
  return scrollHeight - scrollTop - clientHeight <= STICK_EPSILON_PX
}

export interface WindowBounds {
  start: number
  end: number
  newer: number
}

export function windowBounds(
  shownLength: number,
  anchor: number | null,
  renderWindow: number,
): WindowBounds {
  const end = anchor === null ? shownLength : Math.min(anchor, shownLength)
  return {
    start: Math.max(0, end - renderWindow),
    end,
    newer: Math.max(0, shownLength - end),
  }
}

export function anchorIndex(seqs: number[], anchorSeq: number | null): number | null {
  if (anchorSeq === null) return null
  if (seqs.length === 0 || seqs[0] > anchorSeq) return null
  let count = 0
  while (count < seqs.length && seqs[count] <= anchorSeq) count += 1
  return count
}
