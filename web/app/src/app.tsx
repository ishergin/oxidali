import './app.css'
import { useState } from 'preact/hooks'
import { connect, connection, connectionReason } from './api/ws'
import { controllerRole } from './api/client'
import { route } from './router'
import { ObservationProvider } from './observation'
import { OperationToasts } from './toast'
import { Console } from './screens/console'
import { Dashboard } from './screens/dashboard'
import { DiagnosticsScreen } from './screens/diagnostics'
import { FirmwareScreen } from './screens/firmware'
import { DeviceDetail } from './screens/device-detail'
import { Devices } from './screens/devices'
import { InputDeviceDetailScreen } from './screens/input-device-detail'
import { InputDevicesScreen } from './screens/input-devices'
import { Groups } from './screens/groups'
import { HclScheduleEditor, HclSchedules } from './screens/hcl'
import { Lamps } from './screens/lamps'
import { Operations } from './screens/operations'
import { RulesScreen } from './screens/rules'
import { Scenes } from './screens/scenes'
import { SettingsHomeAssistant } from './screens/settings-home-assistant'
import { PoliciesScreen } from './screens/policies'
import { SettingsDali } from './screens/settings-dali'
import { SettingsTime } from './screens/settings-time'
import { SettingsRedundancy } from './screens/settings-redundancy'
import { SettingsPoller } from './screens/settings-poller'
import { Logs } from './screens/logs'
import { Sniffer } from './screens/sniffer'
import { StatsScreen } from './screens/stats'

connect()

interface NavItem {
  href: string
  icon: string
  label: string
  match: string[]
}

const NAV_GROUPS: { title: string; items: NavItem[] }[] = [
  {
    title: 'Control',
    items: [
      { href: '#/', icon: '▦', label: 'Dashboard', match: ['dashboard'] },
      { href: '#/devices', icon: '◧', label: 'Physical devices', match: ['devices', 'device'] },
      { href: '#/lamps', icon: '◍', label: 'Virtual lamps', match: ['lamps'] },
      { href: '#/input-devices', icon: '⌘', label: 'Input devices', match: ['input-devices', 'input-device'] },
      { href: '#/groups', icon: '⊞', label: 'Groups', match: ['groups'] },
      { href: '#/scenes', icon: '✦', label: 'Scenes', match: ['scenes', 'scene'] },
      { href: '#/hcl', icon: '◐', label: 'HCL schedules', match: ['hcl', 'hcl-schedule'] },
      { href: '#/rules', icon: '∴', label: 'Rules', match: ['rules'] },
    ],
  },
  {
    title: 'System',
    items: [
      { href: '#/operations', icon: '≡', label: 'Operations', match: ['operations', 'operation'] },
      { href: '#/stats', icon: '▤', label: 'Stats', match: ['stats'] },
      { href: '#/console', icon: '›_', label: 'DALI console', match: ['console'] },
      { href: '#/sniffer', icon: '∿', label: 'DALI sniffer', match: ['sniffer'] },
      { href: '#/logs', icon: '☰', label: 'Log', match: ['logs'] },
      { href: '#/diagnostics', icon: '◔', label: 'Diagnostics', match: ['diagnostics'] },
      { href: '#/firmware', icon: '⇧', label: 'Firmware', match: ['firmware'] },
    ],
  },
  {
    title: 'Settings',
    items: [
      { href: '#/settings/poller', icon: '⟳', label: 'Poller', match: ['settings-poller'] },
      { href: '#/settings/dali', icon: '◈', label: 'DALI', match: ['settings-dali'] },
      { href: '#/settings/time', icon: '◷', label: 'Time', match: ['settings-time'] },
      { href: '#/settings/home-assistant', icon: '⌂', label: 'Home Assistant', match: ['settings-home-assistant'] },
      { href: '#/settings/redundancy', icon: '⇄', label: 'Redundancy', match: ['settings-redundancy'] },
      { href: '#/policies', icon: '⛨', label: 'Policies', match: ['policies'] },
    ],
  },
]

type Theme = 'dark' | 'light'
const THEME_KEY = 'dali2rust-theme'

function initialTheme(): Theme {
  try {
    const saved = localStorage.getItem(THEME_KEY)
    if (saved === 'light' || saved === 'dark') return saved
  } catch {
  }
  return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'
}

document.documentElement.dataset.theme = initialTheme()

const CONN_LABEL: Record<string, string> = {
  live: 'Live',
  down: 'Offline',
  refused: 'Refused',
}

function ConnectionDot() {
  const state = connection.value
  const cls = state === 'live' || state === 'down' || state === 'refused' ? state : 'retry'
  const label = CONN_LABEL[state] ?? 'Reconnecting'
  return (
    <span class={`conn ${cls}`} title={connectionReason.value}>
      <span class="dot" />
      <span class="conn-label">{label}</span>
    </span>
  )
}

function RoleChip() {
  if (controllerRole.value !== 'standby') return null
  return (
    <a
      class="rolechip"
      href="#/settings/redundancy"
      title="This controller is not driving the bus. Reads work; anything that touches the wire is refused."
    >
      standby
    </a>
  )
}

function ThemeToggle() {
  const [theme, setTheme] = useState<Theme>(
    document.documentElement.dataset.theme === 'light' ? 'light' : 'dark',
  )
  const toggle = () => {
    const next: Theme = theme === 'dark' ? 'light' : 'dark'
    document.documentElement.dataset.theme = next
    try {
      localStorage.setItem(THEME_KEY, next)
    } catch {
    }
    setTheme(next)
  }
  return (
    <button class="btn ghost sm themebtn" onClick={toggle} title="Toggle color theme">
      {theme === 'dark' ? '☾ Dark' : '☀ Light'}
    </button>
  )
}

function Screen() {
  const r = route.value
  switch (r.name) {
    case 'dashboard':
      return <Dashboard />
    case 'devices':
      return <Devices />
    case 'device':
      return (
        <DeviceDetail key={r.params.short} short={Number(r.params.short)} tab={r.params.tab} />
      )
    case 'input-devices':
      return <InputDevicesScreen />
    case 'input-device':
      return (
        <InputDeviceDetailScreen
          key={r.params.short}
          short={Number(r.params.short)}
          instance={r.params.instance}
        />
      )
    case 'lamps':
      return <Lamps />
    case 'groups':
      return <Groups />
    case 'scenes':
      return <Scenes key="0" sceneId={0} />
    case 'scene':
      return <Scenes key={r.params.id} sceneId={Number(r.params.id)} />
    case 'hcl':
      return <HclSchedules />
    case 'hcl-schedule':
      return <HclScheduleEditor key={r.params.id} id={r.params.id} />
    case 'rules':
      return <RulesScreen />
    case 'operations':
      return <Operations />
    case 'operation':
      return <Operations selectedId={r.params.id} />
    case 'console':
      return <Console />
    case 'sniffer':
      return <Sniffer />
    case 'logs':
      return <Logs />
    case 'stats':
      return <StatsScreen />
    case 'diagnostics':
      return <DiagnosticsScreen />
    case 'firmware':
      return <FirmwareScreen />
    case 'settings-dali':
      return <SettingsDali />
    case 'settings-time':
      return <SettingsTime />
    case 'settings-poller':
      return <SettingsPoller />
    case 'settings-home-assistant':
      return <SettingsHomeAssistant />
    case 'settings-redundancy':
      return <SettingsRedundancy />
    case 'policies':
      return <PoliciesScreen />
  }
}

export function App() {
  const current = route.value.name
  return (
    <ObservationProvider>
    <div class="shell">
      <aside class="side">
        <div class="brand">
          <span class="glyph">◍</span>
          <span class="name">dali2rust</span>
          <RoleChip />
        </div>
        <nav class="nav">
          {NAV_GROUPS.map((group) => (
            <div class="navgroup" key={group.title}>
              <div class="gh">{group.title}</div>
              {group.items.map((item) => (
                <a
                  key={item.href}
                  href={item.href}
                  class={item.match.includes(current) ? 'active' : undefined}
                >
                  <span class="ic">{item.icon}</span>
                  {item.label}
                </a>
              ))}
            </div>
          ))}
        </nav>
        <div class="side-foot">
          <ConnectionDot />
          <ThemeToggle />
        </div>
      </aside>
      <div class="main">
        <Screen />
      </div>
      <OperationToasts />
    </div>
    </ObservationProvider>
  )
}
