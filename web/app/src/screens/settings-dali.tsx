import { api } from '../api/client'
import { Card, FieldRow, Switch, EditableText } from '../components/ui'
import { useSettingsDraft } from '../hooks'

export function SettingsDali() {
  const { data, current, busy, setDraft, discard, save } = useSettingsDraft(() =>
    api.daliSettings(),
  )

  if (!data) return <div class="empty">Loading DALI settings…</div>
  const s = current ?? data
  const dirty =
    s.dt8_auto_activation_repair !== data.dt8_auto_activation_repair ||
    s.dt8_rgbwaf_control_assert !== data.dt8_rgbwaf_control_assert ||
    s.application_active !== data.application_active ||
    s.device_short_address !== data.device_short_address

  const apply = () =>
    void save('DALI settings', () =>
      api.patchDaliSettings({
        dt8_auto_activation_repair: s.dt8_auto_activation_repair,
        dt8_rgbwaf_control_assert: s.dt8_rgbwaf_control_assert,
        application_active: s.application_active,
        device_short_address: s.device_short_address,
      }),
    )

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
              setDraft({ ...s, dt8_auto_activation_repair: !s.dt8_auto_activation_repair })
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
              setDraft({ ...s, dt8_rgbwaf_control_assert: !s.dt8_rgbwaf_control_assert })
            }
          />
        </FieldRow>
      </Card>

      <Card title="Application controller (R16-C)">
        <FieldRow
          label="Active"
          hint="103 §9.9.1 applicationActive: off = this controller puts no forward frame on the wire — the standard's standby state. Another controller's ENABLE/DISABLE pair flips it too."
        >
          <Switch
            on={s.application_active}
            onToggle={() => setDraft({ ...s, application_active: !s.application_active })}
          />
        </FieldRow>
        <FieldRow
          label="Device short address"
          hint="Our own control-device address (0–63, empty = none). Without one, only broadcast ENABLE/DISABLE reaches us."
        >
          <EditableText
            value={s.device_short_address === null ? '' : String(s.device_short_address)}
            placeholder="—"
            onCommit={(raw: string) => {
              const t = raw.trim()
              if (t === '') return setDraft({ ...s, device_short_address: null })
              const n = Number(t)
              if (!Number.isInteger(n) || n < 0 || n > 63) return
              setDraft({ ...s, device_short_address: n })
            }}
          />
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
