import type { HclSchedulePoint } from '../api/types.js'

export interface CurveSample {
  offset_minutes: number
  value: number | null
}

export type CurveVertex = readonly [minutes: number, value: number]

export const DAY_END_MINUTES = 1440

export function levelSamples(sortedAbsolute: HclSchedulePoint[]): CurveSample[] {
  return sortedAbsolute.map((p) => ({
    offset_minutes: p.offset_minutes,
    value: p.level_mode === 'absolute' ? p.level : null,
  }))
}

export function cctSamples(sortedAbsolute: HclSchedulePoint[]): CurveSample[] {
  return sortedAbsolute.map((p) => ({
    offset_minutes: p.offset_minutes,
    value: p.color_temperature_kelvin,
  }))
}

export function curveLines(sortedSamples: CurveSample[], stepped: boolean): CurveVertex[][] {
  const lines: CurveVertex[][] = []
  let line: CurveVertex[] = []
  for (const { offset_minutes: at, value } of sortedSamples) {
    const held = line.length > 0 ? line[line.length - 1][1] : null
    if (value == null) {
      if (held != null) {
        line.push([at, held])
        lines.push(line)
      }
      line = []
      continue
    }
    if (held != null && stepped) line.push([at, held])
    line.push([at, value])
  }
  if (line.length > 0) {
    line.push([DAY_END_MINUTES, line[line.length - 1][1]])
    lines.push(line)
  }
  return lines
}
