import { signal } from '@preact/signals'

export interface Route {
  name:
    | 'dashboard'
    | 'devices'
    | 'device'
    | 'input-devices'
    | 'input-device'
    | 'lamps'
    | 'groups'
    | 'scenes'
    | 'scene'
    | 'operations'
    | 'operation'
    | 'hcl'
    | 'hcl-schedule'
    | 'rules'
    | 'console'
    | 'sniffer'
  | 'logs'
    | 'files'
    | 'stats'
    | 'diagnostics'
    | 'firmware'
    | 'settings-poller'
    | 'settings-dali'
    | 'settings-redundancy'
    | 'policies'
    | 'settings-home-assistant'
  params: Record<string, string>
}

const ROUTES: { re: RegExp; name: Route['name']; keys: string[] }[] = [
  { re: /^\/$/, name: 'dashboard', keys: [] },
  { re: /^\/devices$/, name: 'devices', keys: [] },
  { re: /^\/devices\/(\d+)$/, name: 'device', keys: ['short'] },
  { re: /^\/devices\/(\d+)\/([a-z-]+)$/, name: 'device', keys: ['short', 'tab'] },
  { re: /^\/input-devices$/, name: 'input-devices', keys: [] },
  { re: /^\/input-devices\/(\d+)$/, name: 'input-device', keys: ['short'] },
  { re: /^\/input-devices\/(\d+)\/(\d+)$/, name: 'input-device', keys: ['short', 'instance'] },
  { re: /^\/lamps$/, name: 'lamps', keys: [] },
  { re: /^\/groups$/, name: 'groups', keys: [] },
  { re: /^\/scenes$/, name: 'scenes', keys: [] },
  { re: /^\/scenes\/(\d+)$/, name: 'scene', keys: ['id'] },
  { re: /^\/hcl$/, name: 'hcl', keys: [] },
  { re: /^\/hcl\/([a-z0-9_-]+)$/, name: 'hcl-schedule', keys: ['id'] },
  { re: /^\/rules$/, name: 'rules', keys: [] },
  { re: /^\/operations$/, name: 'operations', keys: [] },
  { re: /^\/operations\/([^/]+)$/, name: 'operation', keys: ['id'] },
  { re: /^\/console$/, name: 'console', keys: [] },
  { re: /^\/sniffer$/, name: 'sniffer', keys: [] },
  { re: /^\/logs$/, name: 'logs', keys: [] },
  { re: /^\/files$/, name: 'files', keys: [] },
  { re: /^\/stats$/, name: 'stats', keys: [] },
  { re: /^\/diagnostics$/, name: 'diagnostics', keys: [] },
  { re: /^\/firmware$/, name: 'firmware', keys: [] },
  { re: /^\/settings\/poller$/, name: 'settings-poller', keys: [] },
  { re: /^\/settings\/dali$/, name: 'settings-dali', keys: [] },
  { re: /^\/settings\/redundancy$/, name: 'settings-redundancy', keys: [] },
  { re: /^\/policies$/, name: 'policies', keys: [] },
  { re: /^\/settings\/home-assistant$/, name: 'settings-home-assistant', keys: [] },
]

function parse(): Route {
  const hash = location.hash.replace(/^#/, '') || '/'
  for (const { re, name, keys } of ROUTES) {
    const m = hash.match(re)
    if (m) {
      const params: Record<string, string> = {}
      keys.forEach((k, i) => {
        params[k] = decodeURIComponent(m[i + 1])
      })
      return { name, params }
    }
  }
  return { name: 'dashboard', params: {} }
}

export const route = signal<Route>(parse())

window.addEventListener('hashchange', () => {
  route.value = parse()
})

export function nav(path: string) {
  location.hash = `#${path}`
}
