import { api } from '../api/client'
import type { DisplayPage, DisplaySettings } from '../api/types'
import { Card, EditableText, FieldRow, SelChip, Switch } from '../components/ui'
import { hex2 } from '../format'
import { useSettingsDraft } from '../hooks'
import {
  DISPLAY_PAGES,
  displayPatch,
  SCREENSAVER_OFF_S,
  screensaverSeconds,
  withPageToggled,
} from './display-settings'

const I2C_ADDRESSES = [0x3c, 0x3d]

const ROTATIONS = [0, 180]

const PAGE_SECONDS = 5

const PAGE_LABEL: Record<DisplayPage, string> = {
  summary: 'Summary',
  network: 'Network',
  bus: 'Bus',
  activity: 'Activity',
}

const PAGE_SHOWS: Record<DisplayPage, string> = {
  summary: 'address, devices, bus, HCL, services, last input, live action',
  network: 'IP, role, uptime, time sync, MQTT, WebSocket clients, flash save state',
  bus: 'bus verdict, frames and foreign frames per minute, wire load, poller, gear counts, load histogram',
  activity: 'running operation, last action with its source and age, last input event, HCL',
}

const SCREENSAVER_PRESETS: { s: number; label: string }[] = [
  { s: SCREENSAVER_OFF_S, label: 'Off' },
  { s: 60, label: '1 min' },
  { s: 300, label: '5 min' },
  { s: 900, label: '15 min' },
  { s: 3600, label: '1 h' },
]

const pagesHint = (
  <>
    At least one. The panel shows each selected page for {PAGE_SECONDS} s, always in this order;
    with one page it stays put.
    {DISPLAY_PAGES.map((p) => (
      <span key={p}>
        {' '}
        <b>{PAGE_LABEL[p]}</b>: {PAGE_SHOWS[p]}.
      </span>
    ))}
  </>
)

export function SettingsDisplay() {
  const { data, current, busy, setDraft, discard, save } = useSettingsDraft(() =>
    api.displaySettings(),
  )

  if (!data) return <div class="empty">Loading display settings…</div>
  const s = current ?? data
  const edit = (patch: Partial<DisplaySettings>) => setDraft({ ...s, ...patch })

  const body = displayPatch(s, data)
  const dirty = Object.keys(body).length
  const pagesEmpty = s.pages.length === 0

  const apply = () => save('Display settings', () => api.patchDisplaySettings(body))

  return (
    <div>
      <div class="head">
        <h1>Display</h1>
        <span class="sub">
          The OLED status panel on the controller: whether it is lit, how it is wired and
          mounted, and which pages it turns through.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty} onClick={discard}>
          Discard
        </button>
        <button class="btn primary" disabled={busy || !dirty || pagesEmpty} onClick={apply}>
          {dirty ? `Apply (${dirty})` : 'Apply'}
        </button>
      </div>

      <Card title="Panel">
        <FieldRow
          label="Enabled"
          hint="On by default. Off powers the panel down and draws nothing; the controller keeps running."
        >
          <Switch on={s.enabled} onToggle={() => edit({ enabled: !s.enabled })} />
        </FieldRow>

        <FieldRow
          label="I2C address"
          hint="The two SSD1306 addresses; the jumper on the panel module decides which one it answers. The wrong one leaves the panel blank."
        >
          <div class="presets">
            {I2C_ADDRESSES.map((a) => (
              <SelChip key={a} on={s.i2c_address === a} onClick={() => edit({ i2c_address: a })}>
                {hex2(a)}
              </SelChip>
            ))}
          </div>
        </FieldRow>

        <FieldRow
          label="Rotation"
          hint="180° for a panel mounted upside down on the rail. Portrait is not offered: the 21-column summary does not fit a panel 64 pixels wide."
        >
          <div class="presets">
            {ROTATIONS.map((r) => (
              <SelChip key={r} on={s.rotation_deg === r} onClick={() => edit({ rotation_deg: r })}>
                {`${r}°`}
              </SelChip>
            ))}
          </div>
        </FieldRow>

        <FieldRow
          label="Screensaver"
          hint="Off, or 10 s … 24 h. After this long with nothing new to show the panel powers down; a new action other than an HCL step or a background read, a button or slider, an operation, a failure or a live fault (link down, bus alarm, faulty gear) wakes it."
        >
          <div class="presets">
            {SCREENSAVER_PRESETS.map((p) => (
              <SelChip
                key={p.s}
                on={s.screensaver_timeout_s === p.s}
                onClick={() => edit({ screensaver_timeout_s: p.s })}
              >
                {p.label}
              </SelChip>
            ))}
          </div>
          <EditableText
            cls="num"
            inputMode="numeric"
            value={String(s.screensaver_timeout_s)}
            onCommit={(raw) =>
              edit({ screensaver_timeout_s: screensaverSeconds(raw, s.screensaver_timeout_s) })
            }
          />
          <span class="unit">s</span>
        </FieldRow>
      </Card>

      <Card title="Pages">
        <FieldRow label="Pages" hint={pagesHint}>
          <div class="presets">
            {DISPLAY_PAGES.map((p) => (
              <SelChip
                key={p}
                on={s.pages.includes(p)}
                onClick={() => edit({ pages: withPageToggled(s.pages, p) })}
              >
                {PAGE_LABEL[p]}
              </SelChip>
            ))}
          </div>
          {pagesEmpty ? <p class="req">Pick at least one page.</p> : null}
        </FieldRow>

        <FieldRow
          label="Page time"
          hint="How long each page holds the panel before the next one. Not configurable."
        >
          <span class="unit">{PAGE_SECONDS} s</span>
        </FieldRow>
      </Card>
    </div>
  )
}
