import { useEffect, useRef, useState } from 'preact/hooks'
import { api } from '../api/client'
import type { PhysicalDeviceSummary, VirtualLamp, VirtualLampPatch } from '../api/types'
import { EditableName, LampState, LevelSlider, Switch } from '../components/ui'
import { ADAPTER, pad2 } from '../format'
import { useLive } from '../hooks'
import { errorMessage, mutate, notify } from '../toast'

const CONFIRM_MS = 3000
const VL_ID_MAX = 63

interface FreeDevice {
  sa: number
  label: string
}

function deviceOptionLabel(dev: PhysicalDeviceSummary): string {
  const name = dev.name ? ` · ${dev.name}` : ''
  return `SA ${pad2(dev.short_address)} · ${dev.device_type_effective}${name}`
}

function LampRow({
  lamp,
  freeDevices,
  onDone,
}: {
  lamp: VirtualLamp
  freeDevices: FreeDevice[]
  onDone: () => void
}) {
  const id = lamp.virtual_lamp_id
  const sa = lamp.binding?.physical_short_address ?? null

  const [editingBind, setEditingBind] = useState(false)
  const [bindSel, setBindSel] = useState('')
  const [armed, setArmed] = useState<'unbind' | 'delete' | null>(null)
  const confirmTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(
    () => () => {
      if (confirmTimer.current != null) clearTimeout(confirmTimer.current)
    },
    [],
  )
  const arm = (what: 'unbind' | 'delete') => {
    if (confirmTimer.current != null) clearTimeout(confirmTimer.current)
    setArmed(what)
    confirmTimer.current = setTimeout(() => setArmed(null), CONFIRM_MS)
  }
  const disarm = () => {
    if (confirmTimer.current != null) clearTimeout(confirmTimer.current)
    setArmed(null)
  }

  const saveName = async (name: string) => {
    if (name === lamp.name) return
    await mutate('Rename lamp', () => api.patchVirtualLamp(ADAPTER, id, { name }), onDone)
  }

  const bindOptions: FreeDevice[] =
    sa != null
      ? [{ sa, label: `SA ${pad2(sa)} · current` }, ...freeDevices]
      : freeDevices

  const openBind = () => {
    setBindSel(String(sa ?? bindOptions[0]?.sa ?? ''))
    setEditingBind(true)
  }

  const applyBind = async () => {
    const target = Number(bindSel)
    if (!Number.isInteger(target)) return
    setEditingBind(false)
    if (target === sa) return
    await mutate('Bind lamp', () => api.bindVirtualLamp(ADAPTER, id, target), onDone)
  }

  const unbind = async () => {
    if (armed !== 'unbind') {
      arm('unbind')
      return
    }
    disarm()
    await mutate('Unbind lamp', () => api.unbindVirtualLamp(ADAPTER, id), onDone)
  }

  const remove = async () => {
    if (armed !== 'delete') {
      arm('delete')
      return
    }
    disarm()
    await mutate('Delete lamp', () => api.deleteVirtualLamp(ADAPTER, id), onDone)
  }

  const patchLamp = async (title: string, body: VirtualLampPatch) => {
    try {
      await api.patchVirtualLamp(ADAPTER, id, body)
      onDone()
    } catch (e) {
      notify(title, 'failed', errorMessage(e))
    }
  }

  const setLevel = async (level: number) => {
    try {
      await api.lampTargetState(
        ADAPTER,
        id,
        level === 0 ? { power: 'off' } : { power: 'on', level },
      )
      onDone()
    } catch (e) {
      notify('Set level', 'failed', errorMessage(e))
    }
  }

  return (
    <tr>
      <td>
        <span class="vlid">VL {pad2(id)}</span>
      </td>
      <td>
        <EditableName
          cls={`name-edit${lamp.name ? '' : ' name-faint'}`}
          value={lamp.name}
          placeholder="— unnamed"
          onCommit={(v) => void saveName(v)}
        />
      </td>
      <td>
        {editingBind ? (
          <span class="bind-edit">
            <select
              class="sel"
              value={bindSel}
              onChange={(e) => setBindSel(e.currentTarget.value)}
            >
              {bindOptions.map((o) => (
                <option key={o.sa} value={String(o.sa)}>
                  {o.label}
                </option>
              ))}
            </select>
            <button class="btn sm primary" onClick={applyBind} disabled={bindSel === ''}>
              Apply
            </button>
            <button class="btn sm ghost" onClick={() => setEditingBind(false)}>
              Cancel
            </button>
          </span>
        ) : sa != null ? (
          <span class="chip bind">SA {pad2(sa)}</span>
        ) : (
          <span class="chip unbound">unbound</span>
        )}
      </td>
      <td>
        <Switch on={lamp.ha_entity_enabled} onToggle={() => void patchLamp('HA entity', { ha_entity_enabled: !lamp.ha_entity_enabled })} />
      </td>
      <td>
        <LampState state={lamp.state} />
      </td>
      <td>
        <LevelSlider readout mini value={lamp.state.level ?? 0} onCommit={setLevel} />
      </td>
      <td class="acts-col">
        <span class="acts">
          {!editingBind && (
            <button
              class="btn sm ghost"
              onClick={openBind}
              disabled={bindOptions.length === 0}
              title={bindOptions.length === 0 ? 'No free physical devices' : undefined}
            >
              {sa != null ? 'Rebind…' : 'Bind…'}
            </button>
          )}
          {sa != null && (
            <button class={`btn sm ghost${armed === 'unbind' ? ' sure' : ''}`} onClick={unbind}>
              {armed === 'unbind' ? 'Sure?' : 'Unbind'}
            </button>
          )}
          <button
            class={`btn sm ghost danger${armed === 'delete' ? ' sure' : ''}`}
            onClick={remove}
            title="Forget this lamp: name, Home Assistant entity, binding, group row and scene rows. The gear keeps whatever is already programmed into it."
          >
            {armed === 'delete' ? 'Sure?' : 'Delete'}
          </button>
        </span>
      </td>
    </tr>
  )
}

function NewLampRow({
  freeIds,
  freeDevices,
  onCancel,
  onDone,
}: {
  freeIds: number[]
  freeDevices: FreeDevice[]
  onCancel: () => void
  onDone: () => void
}) {
  const [vlId, setVlId] = useState(String(freeIds[0] ?? ''))
  const [devSa, setDevSa] = useState(String(freeDevices[0]?.sa ?? ''))

  const create = async () => {
    await mutate('New lamp', () => api.bindVirtualLamp(ADAPTER, Number(vlId), Number(devSa)), onDone)
  }

  return (
    <tr class="editor-row">
      <td colSpan={7}>
        <span class="bind-edit">
          <span class="lbl">New virtual lamp</span>
          <select class="sel" value={vlId} onChange={(e) => setVlId(e.currentTarget.value)}>
            {freeIds.map((i) => (
              <option key={i} value={String(i)}>
                VL {pad2(i)}
              </option>
            ))}
          </select>
          <span class="lbl">bind to</span>
          <select class="sel" value={devSa} onChange={(e) => setDevSa(e.currentTarget.value)}>
            {freeDevices.map((o) => (
              <option key={o.sa} value={String(o.sa)}>
                {o.label}
              </option>
            ))}
          </select>
          <button
            class="btn sm primary"
            onClick={create}
            disabled={vlId === '' || devSa === ''}
          >
            Create
          </button>
          <button class="btn sm ghost" onClick={onCancel}>
            Cancel
          </button>
        </span>
      </td>
    </tr>
  )
}

export function Lamps() {
  const { data, reload } = useLive(async () => {
    const [lamps, adapters, devices] = await Promise.all([
      api.virtualLamps(ADAPTER),
      api.adapters(),
      api.physicalDevices(ADAPTER),
    ])
    return {
      lamps: lamps.virtual_lamps,
      limit: adapters.adapters[0]?.limits.virtual_lamps ?? 64,
      devices: devices.physical_devices,
    }
  }, ['virtual_lamps', 'physical_devices'])
  const [creating, setCreating] = useState(false)
  const [showUnbound, setShowUnbound] = useState(false)

  const lamps = data?.lamps ?? []
  const boundSas = new Set(
    lamps
      .map((l) => l.binding?.physical_short_address)
      .filter((sa): sa is number => sa != null),
  )
  const freeDevices: FreeDevice[] = (data?.devices ?? [])
    .filter((d) => !boundSas.has(d.short_address))
    .map((d) => ({ sa: d.short_address, label: deviceOptionLabel(d) }))
  const usedIds = new Set(lamps.map((l) => l.virtual_lamp_id))
  const freeIds = Array.from({ length: VL_ID_MAX + 1 }, (_, i) => i).filter(
    (i) => !usedIds.has(i),
  )

  const bound = lamps.filter((l) => l.binding != null)
  const unbound = lamps.filter((l) => l.binding == null)

  const rowDone = () => void reload()

  return (
    <>
      <div class="crumbs">
        <a href="#/">Adapter {ADAPTER}</a> / Virtual lamps
      </div>
      <div class="head">
        <h1>Virtual lamps</h1>
        <span class="sub">{data ? `${lamps.length} of ${data.limit} configured` : '…'}</span>
        <span class="spacer" />
        <button
          class="btn primary"
          onClick={() => setCreating(true)}
          disabled={creating || freeIds.length === 0 || freeDevices.length === 0}
          title={
            freeDevices.length === 0 ? 'No free physical devices to bind' : undefined
          }
        >
          + New virtual lamp
        </button>
      </div>
      <p class="hint">
        <b>Unbound lamps stage state:</b> level and color set on an unbound lamp are kept in the
        registry and pushed to hardware when the lamp is bound to a short address.
      </p>

      <div class="table-wrap">
        <table>
          <colgroup>
            <col style="width:64px" />
            <col />
            <col style="width:88px" />
            <col style="width:96px" />
            <col style="width:150px" />
            <col style="width:210px" />
            <col style="width:236px" />
          </colgroup>
          <thead>
            <tr>
              <th>VL</th>
              <th>Name</th>
              <th>Binding</th>
              <th>HA entity</th>
              <th>State</th>
              <th>Level</th>
              <th class="acts-col"></th>
            </tr>
          </thead>
          <tbody>
            {creating && (
              <NewLampRow
                freeIds={freeIds}
                freeDevices={freeDevices}
                onCancel={() => setCreating(false)}
                onDone={() => {
                  setCreating(false)
                  void reload()
                }}
              />
            )}
            {bound.map((lamp) => (
              <LampRow
                key={lamp.virtual_lamp_id}
                lamp={lamp}
                freeDevices={freeDevices}
                onDone={rowDone}
              />
            ))}
            {unbound.length > 0 && (
              <tr class="group-row" onClick={() => setShowUnbound(!showUnbound)}>
                <td colSpan={7}>
                  <span class="tri">{showUnbound ? '▾' : '▸'}</span> Unbound virtual lamps (
                  {unbound.length})
                </td>
              </tr>
            )}
            {showUnbound &&
              unbound.map((lamp) => (
                <LampRow
                  key={lamp.virtual_lamp_id}
                  lamp={lamp}
                  freeDevices={freeDevices}
                  onDone={rowDone}
                />
              ))}
          </tbody>
        </table>
        {data && lamps.length === 0 && !creating && (
          <div class="empty">No virtual lamps configured — create one to bind a physical device.</div>
        )}
      </div>
    </>
  )
}
