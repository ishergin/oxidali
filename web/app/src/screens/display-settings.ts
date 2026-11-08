import type { DisplayPage, DisplaySettings } from '../api/types.js'
import { withGroupToggled } from './poller-groups.js'

export const DISPLAY_PAGES: readonly DisplayPage[] = ['summary', 'network', 'bus', 'activity']

export const SCREENSAVER_OFF_S = 0
export const SCREENSAVER_MIN_S = 10
export const SCREENSAVER_MAX_S = 86_400

const KEYS: readonly (keyof DisplaySettings)[] = [
  'enabled',
  'i2c_address',
  'rotation_deg',
  'screensaver_timeout_s',
  'pages',
]

export function screensaverSeconds(raw: string, fallback: number): number {
  const n = Number.parseInt(raw, 10)
  if (Number.isNaN(n)) return fallback
  if (n <= SCREENSAVER_OFF_S) return SCREENSAVER_OFF_S
  return Math.min(SCREENSAVER_MAX_S, Math.max(SCREENSAVER_MIN_S, n))
}

export function withPageToggled(pages: readonly DisplayPage[], page: DisplayPage): DisplayPage[] {
  return withGroupToggled(pages, page, DISPLAY_PAGES)
}

export function displayPatch(
  next: DisplaySettings,
  base: DisplaySettings,
): Partial<DisplaySettings> {
  const patch: Partial<DisplaySettings> = {}
  for (const key of KEYS) {
    const same =
      key === 'pages' ? next.pages.join(',') === base.pages.join(',') : next[key] === base[key]
    if (!same) Object.assign(patch, { [key]: next[key] })
  }
  return patch
}
