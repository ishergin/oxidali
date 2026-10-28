import { useEffect, useState } from 'preact/hooks'
import { api } from '../api/client'
import type { PhysicalDeviceSummary, Policies, PolicyException } from '../api/types'
import { Card, FieldRow, Switch, EditableText } from '../components/ui'
import { ADAPTER, pad2 } from '../format'
import { usePoll, useSettingsDraft } from '../hooks'
import { opCommitted, runOp } from '../toast'
import {
  exceptionKey,
  policyOnScreen,
  policyPatch,
  sendOnFreshRead,
  sortedExceptions,
  withEdit,
  type PolicyEdits,
  type PolicyFields,
} from './policies-patch'

const MAX_POLICY_EXCEPTIONS = 32

function ExceptionsCard({
  policy,
  devices,
  onChange,
}: {
  policy: Policies
  devices: PhysicalDeviceSummary[]
  onChange: (exceptions: PolicyException[]) => void
}) {
  const excepted = new Set(policy.exceptions.map(exceptionKey))
  const nameOf = (e: PolicyException) =>
    e.adapter_id === ADAPTER ? devices.find((d) => d.short_address === e.short_address) : undefined
  const candidates = devices.filter(
    (d) => !excepted.has(exceptionKey({ adapter_id: ADAPTER, short_address: d.short_address })),
  )
  const full = policy.exceptions.length >= MAX_POLICY_EXCEPTIONS
  return (
    <Card title="Exceptions">
      {policy.exceptions.map((e) => {
        const device = nameOf(e)
        return (
          <FieldRow
            key={exceptionKey(e)}
            label={`SA ${pad2(e.short_address)}`}
            hint={device ? device.name || '— unnamed' : 'No device at this address'}
          >
            <button
              class="btn sm"
              onClick={() =>
                onChange(policy.exceptions.filter((x) => exceptionKey(x) !== exceptionKey(e)))
              }
            >
              Remove
            </button>
          </FieldRow>
        )
      })}
      <FieldRow
        label="Except a device"
        hint={
          'The policy writes nothing to it, and an apply names it as skipped. The entry ' +
          'follows the device to a new address and leaves with it when the device is ' +
          `forgotten. At most ${MAX_POLICY_EXCEPTIONS}.`
        }
      >
        <select
          class="sel sm"
          value=""
          disabled={full || candidates.length === 0}
          onChange={(ev) => {
            const v = (ev.target as HTMLSelectElement).value
            if (v === '') return
            const added = { adapter_id: ADAPTER, short_address: Number(v) }
            onChange(sortedExceptions([...policy.exceptions, added]))
          }}
        >
          <option value="">device…</option>
          {candidates.map((d) => (
            <option key={d.short_address} value={String(d.short_address)}>
              SA {pad2(d.short_address)}
              {d.name ? ` · ${d.name}` : ''}
            </option>
          ))}
        </select>
      </FieldRow>
    </Card>
  )
}

export function PoliciesScreen() {
  const { data, busy, save } = useSettingsDraft(() => api.policies())
  const { data: list } = usePoll(() => api.physicalDevices(ADAPTER))
  const [edits, setEdits] = useState<PolicyEdits | null>(null)
  useEffect(() => {
    if (edits && data && withEdit(edits, {}, data) === null) setEdits(null)
  }, [edits, data])

  if (!data) return <div class="empty">Loading policies…</div>
  const s = policyOnScreen(edits ?? {}, data)
  const edit = (change: Partial<PolicyFields>) => setEdits(withEdit(edits ?? {}, change, data))
  const discard = () => setEdits(null)
  const dirty = edits !== null && Object.keys(policyPatch(edits, data)).length > 0

  const apply = () =>
    void save('Policies', async () => {
      await sendOnFreshRead(edits ?? {}, api.policies, api.patchPolicies)
      discard()
    })

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
          {level(s.system_failure_level, (n) => edit({ system_failure_level: n }))}
        </FieldRow>
        <FieldRow
          label="Power-on level"
          hint="Where the gear goes when mains returns. Empty = unmanaged. 0–254."
        >
          {level(s.power_on_level, (n) => edit({ power_on_level: n }))}
        </FieldRow>
        <FieldRow
          label="Write to a device as it is discovered"
          hint="Otherwise the policy reaches a device only when you apply it below."
        >
          <Switch
            on={s.apply_on_discovery}
            onToggle={() => edit({ apply_on_discovery: !s.apply_on_discovery })}
          />
        </FieldRow>
      </Card>

      <ExceptionsCard
        policy={s}
        devices={list?.physical_devices ?? []}
        onChange={(exceptions) => edit({ exceptions })}
      />

      <Card title="Write it to the installation">
        <FieldRow
          label="Every known device"
          hint="One write per device, paced, skipping the exceptions. A device that confirms nothing costs one timeout and is reported — silence from a gear the registry believes is present is a fault worth seeing."
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
