import { useEffect, useState } from 'preact/hooks'
import { api } from '../api/client'
import type { BusUnitSetter, DaliSettings } from '../api/types'
import { Card, FieldRow, Switch, EditableText } from '../components/ui'
import { hex6 } from '../format'
import { useSettingsDraft } from '../hooks'
import {
  fieldPatch,
  fieldsOnScreen,
  sendFieldsOnFreshRead,
  withFieldEdit,
  withoutSent,
} from './field-edits'

const SET_BY: Record<BusUnitSetter, { chip: string; text: string }> = {
  wire: { chip: 'chip info', text: 'set by the wire' },
  operator: { chip: 'chip muted', text: 'set by an operator' },
  redundancy: { chip: 'chip muted', text: 'set by redundancy' },
}

function SetBy({ by }: { by: BusUnitSetter | null }) {
  if (by === null) return null
  const { chip, text } = SET_BY[by]
  return <span class={chip}>{text}</span>
}

export function SettingsDali() {
  const { data, busy, save } = useSettingsDraft(() => api.daliSettings())
  const [edits, setEdits] = useState<Partial<DaliSettings> | null>(null)
  useEffect(() => {
    if (!edits || !data) return
    const kept = withFieldEdit(edits, {}, data)
    if (Object.keys(kept ?? {}).length < Object.keys(edits).length) setEdits(kept)
  }, [edits, data])

  if (!data) return <div class="empty">Loading DALI settings…</div>
  const s = fieldsOnScreen(edits ?? {}, data)
  const edit = (change: Partial<DaliSettings>) => setEdits(withFieldEdit(edits ?? {}, change, data))
  const discard = () => setEdits(null)
  const dirty = edits !== null && Object.keys(fieldPatch(edits, data)).length > 0

  const apply = () =>
    void save('DALI settings', async () => {
      const sent = await sendFieldsOnFreshRead(edits ?? {}, api.daliSettings, api.patchDaliSettings)
      setEdits((now) => now && withoutSent(now, sent))
    })

  return (
    <div>
      <div class="head">
        <h1>DALI</h1>
        <span class="sub">
          How this controller drives the bus. One record for the installation; a fixture that
          needs an exception carries it on its own device card.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty} onClick={discard}>
          Discard
        </button>
        <button class="btn primary" disabled={busy || !dirty} onClick={apply}>
          Apply
        </button>
      </div>

      <Card title="Colour">
        <FieldRow
          label="Restore DT8 auto activation"
          hint="When a read finds a gear's Automatic Activation bit clear, put it back before the next colour write. Without it the write is accepted and the colour never moves."
        >
          <Switch
            on={s.dt8_auto_activation_repair}
            onToggle={() =>
              edit({ dt8_auto_activation_repair: !s.dt8_auto_activation_repair })
            }
          />
        </FieldRow>
        <FieldRow
          label="Assert RGBWAF channel control"
          hint="Before a colour write on a six-channel gear, put the control byte into normalised colour control with the driven channels unlinked. Unlike the restore above this chooses a value — the standard's own power-up state has every write ignored."
        >
          <Switch
            on={s.dt8_rgbwaf_control_assert}
            onToggle={() =>
              edit({ dt8_rgbwaf_control_assert: !s.dt8_rgbwaf_control_assert })
            }
          />
        </FieldRow>
      </Card>

      <Card title="Application controller (R16-C)">
        <FieldRow
          label="Active"
          hint="103 §9.9.1 applicationActive: off = this controller puts no forward frame on the wire — the standard's standby state. Another controller's ENABLE/DISABLE pair flips it too."
        >
          <SetBy by={data.set_by.application_active} />
          <Switch
            on={s.application_active}
            onToggle={() => edit({ application_active: !s.application_active })}
          />
        </FieldRow>
        <FieldRow
          label="Device short address"
          hint="Our own control-device address (0–63, empty = none). Without one, only broadcast ENABLE/DISABLE reaches us. It may not be the redundancy peer's."
        >
          <SetBy by={data.set_by.device_short_address} />
          <EditableText
            value={s.device_short_address === null ? '' : String(s.device_short_address)}
            placeholder="—"
            onCommit={(raw: string) => {
              const t = raw.trim()
              if (t === '') return edit({ device_short_address: null })
              const n = Number(t)
              if (!Number.isInteger(n) || n < 0 || n > 63) return
              edit({ device_short_address: n })
            }}
          />
        </FieldRow>
        <FieldRow
          label="Power cycle notification"
          hint="103 §9.12.2: after each boot, announce this controller with POWER NOTIFICATION. Off by default; another controller's ENABLE turns it on too."
        >
          <SetBy by={data.set_by.power_cycle_notification} />
          <Switch
            on={s.power_cycle_notification}
            onToggle={() =>
              edit({ power_cycle_notification: !s.power_cycle_notification })
            }
          />
        </FieldRow>
        <FieldRow
          label="Random address"
          hint="Drawn when another controller sends RANDOMISE; its address search finds this controller by it."
        >
          <SetBy by={data.set_by.random_address} />
          <span class="mono">{hex6(data.random_address)}</span>
        </FieldRow>
        <FieldRow
          label="Device groups"
          hint="The control-device groups another controller added this one to (0–31)."
        >
          <SetBy by={data.set_by.device_groups} />
          <span class="mono">
            {data.device_groups.length === 0 ? 'none' : data.device_groups.join(', ')}
          </span>
        </FieldRow>
        <FieldRow
          label="Operating mode"
          hint="0 is the standard's own mode; only another controller changes it."
        >
          <SetBy by={data.set_by.operating_mode} />
          <span class="mono">{data.operating_mode}</span>
        </FieldRow>
      </Card>

      {!s.application_active && (
        <p class="warnbar">
          Passive: every wire command is refused as controller_passive — buttons,
          schedules and the poller all stop moving light until this is on again.
        </p>
      )}

      {!s.dt8_auto_activation_repair && (
        <p class="warnbar">
          Off: a gear found with the bit clear keeps accepting colour writes that do
          nothing. IEC 62386-209 makes that bit's power-up value <em>set</em>, so leaving
          this on restores the standard's own default rather than imposing one — turn it
          off only for a fixture that stores the byte permanently.
        </p>
      )}
      {!s.dt8_rgbwaf_control_assert && (
        <p class="warnbar">
          Off: a gear whose channels are linked keeps ignoring the colour levels every
          write stages — the standard's own power-up state. Turn this off only for an
          installation whose RGBWAF linkage someone configured deliberately.
        </p>
      )}
    </div>
  )
}
