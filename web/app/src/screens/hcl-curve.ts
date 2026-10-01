export interface CurveLevelPoint {
  offset_minutes: number
  level_mode: string
  level: number | null
}

export function levelRuns<P extends CurveLevelPoint>(sortedPoints: P[]): P[][] {
  const runs: P[][] = []
  let run: P[] = []
  for (const point of sortedPoints) {
    if (point.level_mode === 'absolute' && point.level != null) {
      run.push(point)
      continue
    }
    if (run.length > 0) runs.push(run)
    run = []
  }
  if (run.length > 0) runs.push(run)
  return runs
}
