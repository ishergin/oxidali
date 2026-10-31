export function blurCommits(text: string, atFocus: string | null, cancelled: boolean): boolean {
  return !cancelled && text !== atFocus
}
