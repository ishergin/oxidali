import { api } from '../api/client'
import type { AttributeGroup, PollerSettings } from '../api/types'
import { notifyPollerSettingsChanged } from '../observation'
import { Card, clampInt, EditableText, FieldRow, SelChip, Switch } from '../components/ui'
import { useSettingsDraft } from '../hooks'

const MIN_INTERVAL_MS = 200
const MAX_INTERVAL_MS = 3_600_000

const GROUPS: AttributeGroup[] = [
  'runtime_status',
  'common_102',
  'dt8_color',
  'dt6_led',
  'groups',
  'scenes',
  'extended',
]

const GROUP_LABEL: Record<AttributeGroup, string> = {
  runtime_status: 'Runtime status',
  common_102: 'Common (102)',
  dt8_color: 'DT8 colour',
  dt6_led: 'DT6 LED',
  groups: 'Groups',
  scenes: 'Scenes',
  extended: 'Extended',
}

const PRESETS: { ms: number; label: string }[] = [
  { ms: 1000, label: '1 s' },
  { ms: 5000, label: '5 s' },
  { ms: 30_000, label: '30 s' },
  { ms: 300_000, label: '5 min' },
]

function changedKeys(next: PollerSettings, base: PollerSettings): (keyof PollerSettings)[] {
  const keys: (keyof PollerSettings)[] = [
    'enabled',
    'interval_ms',
    'attribute_groups_default',
    'include_dt8_color',
    'skip_unbound_virtual_lamps',
    'include_energy',
    'include_diagnostics',
  ]
  return keys.filter((k) =>
    k === 'attribute_groups_default'
      ? next[k].join(',') !== base[k].join(',')
      : next[k] !== base[k],
  )
}

function patchBody(
  next: PollerSettings,
  keys: (keyof PollerSettings)[],
): Partial<PollerSettings> {
  const body: Partial<PollerSettings> = {}
  for (const k of keys) Object.assign(body, { [k]: next[k] })
  return body
}

export function SettingsPoller() {
  const { data, current, busy, setDraft, discard, save } = useSettingsDraft(() =>
    api.pollerSettings(),
  )

  if (!data) return <div class="empty">Loading poller settings…</div>
  const s = current ?? data
  const edit = (patch: Partial<PollerSettings>) => setDraft({ ...s, ...patch })

  const dirty = changedKeys(s, data)
  const groupsEmpty = s.attribute_groups_default.length === 0

  const toggleGroup = (g: AttributeGroup) => {
    const on = s.attribute_groups_default.includes(g)
    edit({
      attribute_groups_default: on
        ? s.attribute_groups_default.filter((x) => x !== g)
        : GROUPS.filter((x) => x === g || s.attribute_groups_default.includes(x)),
    })
  }

  const apply = async () => {
    await save('Poller settings', () => api.patchPollerSettings(patchBody(s, dirty)))
    notifyPollerSettingsChanged()
  }

  return (
    <div>
      <div class="head">
        <h1>Poller</h1>
        <span class="sub">
          Background attribute reads that keep registry runtime state fresh without manual
          requests.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty.length} onClick={discard}>
          Discard
        </button>
        <button
          class="btn primary"
          disabled={busy || !dirty.length || groupsEmpty}
          onClick={apply}
        >
          {dirty.length ? `Apply (${dirty.length})` : 'Apply'}
        </button>
      </div>

      <p class="warnbar">
        The poller re-reads every known device on a fixed interval. On a busy
        DALI bus that traffic competes with commands and with any foreign
        master — start slow and widen only if the readings lag.
      </p>

      <Card title="Cycle">
        <FieldRow label="Enabled" hint="Off by default; nothing is polled while this is off.">
          <Switch on={s.enabled} onToggle={() => edit({ enabled: !s.enabled })} />
        </FieldRow>

        <FieldRow label="Interval" hint={`${MIN_INTERVAL_MS} ms … 1 h between cycles.`}>
          <div class="presets">
            {PRESETS.map((p) => (
              <SelChip key={p.ms} on={s.interval_ms === p.ms} onClick={() => edit({ interval_ms: p.ms })}>
                {p.label}
              </SelChip>
            ))}
          </div>
          <EditableText
            cls="num"
            inputMode="numeric"
            value={String(s.interval_ms)}
            onCommit={(raw) =>
              edit({
                interval_ms: clampInt(raw, MIN_INTERVAL_MS, MAX_INTERVAL_MS, s.interval_ms),
              })
            }
          />
          <span class="unit">ms</span>
        </FieldRow>

        <FieldRow
          label="Wire share"
          hint="Background polling never takes more than a quarter of bus time, so an expensive read makes its own period longer. Not configurable."
        >
          <span class="unit">&le; 25 %</span>
        </FieldRow>
      </Card>

      <Card title="What is read">
        <FieldRow label="Attribute groups" hint="At least one. Memory banks are never polled.">
          <div class="presets">
            {GROUPS.map((g) => (
              <SelChip
                key={g}
                on={s.attribute_groups_default.includes(g)}
                onClick={() => toggleGroup(g)}
              >
                {GROUP_LABEL[g]}
              </SelChip>
            ))}
          </div>
          {groupsEmpty ? <p class="req">Pick at least one attribute group.</p> : null}
        </FieldRow>

        <FieldRow
          label="Add DT8 colour"
          hint="Adds the colour group for gear whose effective type is DT8, without asking it of anything else."
        >
          <Switch
            on={s.include_dt8_color}
            onToggle={() => edit({ include_dt8_color: !s.include_dt8_color })}
          />
        </FieldRow>

        <FieldRow
          label="Read energy banks"
          hint="DiiA Part 252 banks 202/203/204, and only for gear that declared device type 51. The gear promises one refresh per 30 s, so power is read no faster than that and the accumulating counters far more rarely."
        >
          <Switch
            on={s.include_energy}
            onToggle={() => edit({ include_energy: !s.include_energy })}
          />
        </FieldRow>

        <FieldRow
          label="Read diagnostics banks"
          hint="DiiA Part 253 banks 205/206/207, for gear that declared device type 52. Dearer than the energy banks — 26 and 30 locations — so it is spaced further apart."
        >
          <Switch
            on={s.include_diagnostics}
            onToggle={() => edit({ include_diagnostics: !s.include_diagnostics })}
          />
        </FieldRow>

        <FieldRow
          label="Skip unbound devices"
          hint="A device no virtual lamp is bound to has no reader, so polling it only costs bus time."
        >
          <Switch
            on={s.skip_unbound_virtual_lamps}
            onToggle={() =>
              edit({ skip_unbound_virtual_lamps: !s.skip_unbound_virtual_lamps })
            }
          />
        </FieldRow>
      </Card>
    </div>
  )
}
