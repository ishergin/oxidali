export const CCT_MIN_K = 2700
export const CCT_MAX_K = 6500

export const CCT_PRODUCT_MIN_K = 1000
export const CCT_PRODUCT_MAX_K = 20000

export const CCT_UNREPORTED_K = 4000

export type CctRange = { min_kelvin: number; max_kelvin: number }

export type CctSliderView = {
  min: number
  max: number
  value: number
  clamped: number | null
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), max)
}

export function cctSliderView(
  observed: number | null | undefined,
  dragging: number | null,
  range?: CctRange | null,
): CctSliderView {
  const min = range?.min_kelvin ?? CCT_MIN_K
  const max = range?.max_kelvin ?? CCT_MAX_K
  const real =
    observed != null && observed >= CCT_PRODUCT_MIN_K && observed <= CCT_PRODUCT_MAX_K
      ? observed
      : null
  const value = clamp(dragging ?? real ?? CCT_UNREPORTED_K, min, max)
  return {
    min,
    max,
    value,
    clamped: dragging === null && real !== null && real !== value ? real : null,
  }
}
