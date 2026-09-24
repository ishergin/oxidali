import { api } from '../api/client'
import { Card, FieldRow, Switch, EditableText } from '../components/ui'
import { useSettingsDraft } from '../hooks'
import { opCommitted, runOp } from '../toast'

export function PoliciesScreen() {
  const { data, current, busy, setDraft, discard, save } = useSettingsDraft(() => api.policies())

  if (!data) return <div class="empty">Loading policies…</div>
  const s = current ?? data
  const dirty =
    s.system_failure_level !== data.system_failure_level ||
    s.power_on_level !== data.power_on_level ||
    s.apply_on_discovery !== data.apply_on_discovery

  const apply = () =>
    void save('Policies', () =>
      api.patchPolicies({
        system_failure_level: s.system_failure_level,
        power_on_level: s.power_on_level,
        apply_on_discovery: s.apply_on_discovery,
      }),
    )

  const writeToAll = () =>
    void (async () => {
      const op = await runOp('Apply policy to every device', () => api.policiesApply())
      if (opCommitted(op)) discard()
    })()

  const level = (value: number | null, set: (n: number | null) => void) => (
    <EditableText
      value={value === null ? '' : String(value)}
      placeholder="unmanaged"
      onCommit={(raw: string) => {
        const t = raw.trim()
        if (t === '') return set(null)
        const n = Number(t)
        if (!Number.isInteger(n) || n < 0 || n > 254) return
        set(n)
      }}
    />
  )

  return (
    <div>
      <div class="head">
        <h1>Policies</h1>
        <span class="sub">
          What a luminaire does when the bus falls silent or mains returns. These are the
          gear's own NVM variables, not settings of ours.
        </span>
        <span class="spacer" />
        <button class="btn" disabled={busy || !dirty} onClick={discard}>
          Discard
        </button>
        <button class="btn primary" disabled={busy || !dirty} onClick={apply}>
          Apply
        </button>
      </div>

      <Card title="Fallback levels">
        <FieldRow
          label="System failure level"
          hint="Where the gear goes when the bus has been silent for more than 500 ms (102 §9.12). Empty = unmanaged: this controller writes it on no device. 0–254."
        >
          {level(s.system_failure_level, (n) => setDraft({ ...s, system_failure_level: n }))}
        </FieldRow>
        <FieldRow
          label="Power-on level"
          hint="Where the gear goes when mains returns. Empty = unmanaged. 0–254."
        >
          {level(s.power_on_level, (n) => setDraft({ ...s, power_on_level: n }))}
        </FieldRow>
        <FieldRow
          label="Write to a device as it is discovered"
          hint="Otherwise the policy reaches a device only when you apply it below."
        >
          <Switch
            on={s.apply_on_discovery}
            onToggle={() => setDraft({ ...s, apply_on_discovery: !s.apply_on_discovery })}
          />
        </FieldRow>
      </Card>

      <Card title="Write it to the installation">
        <FieldRow
          label="Every known device"
          hint="One write per device, paced. A device that confirms nothing costs one timeout and is reported — silence from a gear the registry believes is present is a fault worth seeing."
        >
          <button
            class="btn"
            disabled={busy || dirty || !data.manages_anything}
            onClick={writeToAll}
          >
            Apply to all
          </button>
        </FieldRow>
      </Card>

      {!data.manages_anything && (
        <p class="warnbar">
          Nothing is managed, so there is nothing to write. IEC 62386-102 defaults both
          variables to <span class="mono">0xFE</span> — full brightness — so an installation
          that has never been asked goes to 100 % when the bus dies or the power returns.
        </p>
      )}
      {dirty && (
        <p class="warnbar">
          Unsaved changes. Apply them first: the write below sends what is stored, not what
          is on screen.
        </p>
      )}
    </div>
  )
}
