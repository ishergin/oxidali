import { useState } from 'preact/hooks'
import { api } from '../api/client'
import type { GroupMembershipMatrix } from '../api/types'
import { Chip, EditableName, MatrixCell, Switch } from '../components/ui'
import { ADAPTER, GROUP_COUNT, pad2 } from '../format'
import { useLive } from '../hooks'
import { errorMessage, mutate, notify, saveThenApply, trackOp } from '../toast'

const MATRIX_POLL_MS = 10_000

const editKey = (vl: number, g: number) => `${vl}:${g}`

export function Groups() {
  const { data, reload } = useLive(async () => {
    const [matrix, lamps, groups] = await Promise.all([
      api.groupMatrix(ADAPTER),
      api.virtualLamps(ADAPTER),
      api.groups(ADAPTER),
    ])
    const bound = new Set(
      lamps.virtual_lamps
        .filter((l) => l.binding != null)
        .map((l) => l.virtual_lamp_id),
    )
    const names = new Map(lamps.virtual_lamps.map((l) => [l.virtual_lamp_id, l.name]))
    const shorts = new Map(
      lamps.virtual_lamps
        .filter((l) => l.binding != null)
        .map((l) => [l.virtual_lamp_id, l.binding!.physical_short_address]),
    )
    const haEnabled = new Map(groups.groups.map((g) => [g.group_id, g.ha_entity_enabled]))
    return { matrix, bound, names, shorts, haEnabled }
  }, ['groups', 'virtual_lamps'], { intervalMs: MATRIX_POLL_MS })
  const [edits, setEdits] = useState<Map<string, boolean>>(new Map())
  const [filter, setFilter] = useState('')
  const [busy, setBusy] = useState(false)
  const [renaming, setRenaming] = useState<number | null>(null)

  if (!data) return <div class="empty">Loading group matrix…</div>
  const { matrix, bound, names, shorts, haEnabled } = data

  const desiredOf = (row: GroupMembershipMatrix['rows'][number], g: number) =>
    edits.get(editKey(row.virtual_lamp_id, g)) ?? row.desired[g]

  const rereadGroups = async (vl: number) => {
    const short = shorts.get(vl)
    if (short == null) return
    try {
      const accepted = await api.attributeReads(ADAPTER, short, { attribute_groups: ['groups'] })
      await trackOp(`Re-read groups · SA ${pad2(short)}`, accepted)
      void reload()
    } catch (e) {
      notify('Re-read groups', 'failed', errorMessage(e))
    }
  }

  const toggle = (vl: number, g: number, serverDesired: boolean) => {
    const key = editKey(vl, g)
    setEdits((prev) => {
      const next = new Map(prev)
      const current = next.get(key) ?? serverDesired
      if (!current === serverDesired) next.delete(key)
      else next.set(key, !current)
      return next
    })
  }

  let serverDirty = 0
  for (const row of matrix.rows)
    for (let g = 0; g < GROUP_COUNT; g++) if (row.desired[g] !== row.applied[g]) serverDirty++
  const localAdd = [...edits.entries()].filter(([, v]) => v).length
  const localRm = edits.size - localAdd

  const desiredCount = (g: number) =>
    matrix.rows.reduce((n, row) => n + (desiredOf(row, g) ? 1 : 0), 0)

  const saveGroupName = async (g: number, name: string) => {
    setRenaming(null)
    if (name === (matrix.groups[g]?.name ?? '')) return
    await mutate('Rename group', () => api.patchGroup(ADAPTER, g, { name }), () => {
      void reload()
    })
  }

  const toggleGroupHa = async (g: number, next: boolean) => {
    await mutate('Group Home Assistant', () => api.patchGroup(ADAPTER, g, { ha_entity_enabled: next }), () => {
      void reload()
    })
  }

  const discard = () => setEdits(new Map())

  const applyToBus = async () => {
    const changedVls = new Set([...edits.keys()].map((k) => Number(k.split(':')[0])))
    const rows = matrix.rows
      .filter((r) => changedVls.has(r.virtual_lamp_id))
      .map((r) => ({
        virtual_lamp_id: r.virtual_lamp_id,
        desired: Array.from({ length: GROUP_COUNT }, (_, g) => desiredOf(r, g)),
      }))
    await saveThenApply({
      subject: 'Group',
      setBusy,
      save: edits.size > 0 ? () => api.patchGroupMatrix(ADAPTER, rows) : null,
      apply: () => api.groupsApply(ADAPTER),
      nothingToApply: 'nothing to program',
      onApplied: () => {
        setEdits(new Map())
        void reload()
      },
    })
  }

  const visibleRows = matrix.rows.filter((r) => {
    if (!filter) return true
    const name = names.get(r.virtual_lamp_id) ?? r.name
    return (
      name.toLowerCase().includes(filter.toLowerCase()) ||
      `vl ${pad2(r.virtual_lamp_id)}`.includes(filter.toLowerCase())
    )
  })

  return (
    <>
      <div class="crumbs">Adapter {ADAPTER} / Groups</div>
      <div class="head">
        <h1>Groups</h1>
        <span class="sub">
          Adapter {ADAPTER} · {matrix.rows.length} virtual lamps × {GROUP_COUNT} groups
        </span>
        {serverDirty > 0 && (
          <Chip cls="warn">
            {serverDirty} row{serverDirty === 1 ? '' : 's'} dirty on gear
          </Chip>
        )}
        <span class="spacer" />
        <input
          class="search"
          placeholder="Filter lamps…"
          value={filter}
          onInput={(e) => setFilter(e.currentTarget.value)}
        />
      </div>

      <div class="matrix-wrap">
        <table>
          <thead>
            <tr>
              <th class="lamp">Virtual lamp</th>
              {matrix.groups.map((grp, g) => (
                <th
                  key={g}
                  class={`gname${grp.dirty ? ' dirty' : ''}`}
                  title="Click to rename"
                  onClick={() => {
                    setRenaming((cur) => (cur === null ? g : cur))
                  }}
                >
                  G{g}
                  <span class="cnt">
                    {grp.name || '—'} · {desiredCount(g)}
                  </span>
                  {renaming === g && (
                    <span class="pop" onClick={(e) => e.stopPropagation()}>
                      <span class="ph">Group G{g}</span>
                      <span class="pb">
                        <EditableName
                          autoFocus
                          value={grp.name ?? ''}
                          placeholder={`unnamed`}
                          onCommit={(v) => void saveGroupName(g, v)}
                          onCancel={() => setRenaming(null)}
                          onDismiss={() => setRenaming(null)}
                        />
                        <span class="prow" onMouseDown={(e) => e.preventDefault()}>
                          <span class="pl">Home Assistant</span>
                          <Switch
                            on={haEnabled.get(g) !== false}
                            onToggle={() => void toggleGroupHa(g, haEnabled.get(g) === false)}
                          />
                        </span>
                      </span>
                    </span>
                  )}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {visibleRows.map((row) => {
              const vl = row.virtual_lamp_id
              const isBound = bound.has(vl)
              const name = names.get(vl) || row.name
              const rowDirty =
                row.desired.some((d, g) => d !== row.applied[g]) ||
                [...edits.keys()].some((k) => k.startsWith(`${vl}:`))
              return (
                <tr key={vl} class={rowDirty ? 'rowdirty' : undefined}>
                  <td class="lamp">
                    <span class="vl">VL {pad2(vl)}</span>
                    <span class={`nm${name ? '' : ' name-faint'}`}>{name || '— unnamed'}</span>
                    {!isBound && <span class="unbound">unbound</span>}
                    {rowDirty && isBound && (
                      <button
                        class="act"
                        title="Re-read the gear's group membership — heals a corrupted readback (ISSUE-18)"
                        onClick={() => void rereadGroups(vl)}
                      >
                        re-read
                      </button>
                    )}
                  </td>
                  {Array.from({ length: GROUP_COUNT }, (_, g) => (
                    <td key={g}>
                      <MatrixCell
                        desired={desiredOf(row, g)}
                        applied={row.applied[g]}
                        disabled={!isBound}
                        onToggle={() => toggle(vl, g, row.desired[g])}
                      />
                    </td>
                  ))}
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>

      {edits.size > 0 && (
        <div class="applybar">
          <span class="txt">
            <b>
              {edits.size} pending edit{edits.size === 1 ? '' : 's'}
            </b>{' '}
            — {localAdd} add · {localRm} remove
          </span>
          <button class="btn discard" onClick={discard} disabled={busy}>
            Discard
          </button>
          <button class="btn apply" onClick={applyToBus} disabled={busy}>
            Apply to bus
          </button>
        </div>
      )}
    </>
  )
}
