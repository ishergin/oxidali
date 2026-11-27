import { useEffect, useState } from 'preact/hooks'
import { api } from '../api/client'
import type { GroupMembershipMatrix } from '../api/types'
import { Chip, EditableName, MatrixCell, Switch } from '../components/ui'
import { ADAPTER, GROUP_COUNT, pad2 } from '../format'
import { statusOnly } from '../cluster-feed'
import { useLive } from '../hooks'
import { errorMessage, mutate, notify, saveThenApply, trackOp } from '../toast'
import { applyBar, boundLamps } from './apply-bar'
import { type GroupLock, LOCKS_UNREAD, dropLocked, gearDiffers, groupLocks, lockHint, lockHref, lockLabel } from './groups-view'

const MATRIX_POLL_MS = 10_000

const editKey = (vl: number, g: number) => `${vl}:${g}`

interface LocksRead {
  locks: Map<number, GroupLock>
  unread: boolean
}

const NO_LOCKS: LocksRead = { locks: new Map(), unread: false }

async function readLocks(): Promise<LocksRead> {
  try {
    const settings = await api.clusterSettings()
    const [parts, records] = await Promise.all([
      api.clusterNodeParts('channel', settings.node_number),
      api.clusterRecords('channel'),
    ])
    return { locks: groupLocks(ADAPTER, parts, records), unread: false }
  } catch {
    return { locks: new Map(), unread: true }
  }
}

function useGroupLocks(): LocksRead {
  const { data } = useLive(readLocks, ['cluster'], { intervalMs: MATRIX_POLL_MS, onEvent: statusOnly })
  return data ?? NO_LOCKS
}

function LockedPop({ g, lock, ha, onHa }: { g: number; lock: GroupLock; ha: boolean; onHa: () => void }) {
  return (
    <span class="pop" onClick={(e) => e.stopPropagation()}>
      <span class="ph">
        Group G{g} · in CH{lock.channel}
      </span>
      <span class="pb">
        <span class="pnote">
          Edited in cluster channel {lockLabel(lock)}, not here.{' '}
          <a class="chref" href={lockHref(lock)}>
            Open CH{lock.channel}
          </a>
        </span>
        <span class="prow">
          <span class="pl">Home Assistant</span>
          <Switch on={ha} onToggle={onHa} />
        </span>
      </span>
    </span>
  )
}

function LockedHead(p: {
  g: number
  lock: GroupLock
  count: number
  open: boolean
  ha: boolean
  onOpen: () => void
  onHa: () => void
}) {
  return (
    <th class="gname locked" title={`Linked to ${lockLabel(p.lock)}: click for the channel`} onClick={p.onOpen}>
      G{p.g}
      <span class="cnt">
        CH{p.lock.channel} · {p.count}
      </span>
      {p.open && <LockedPop g={p.g} lock={p.lock} ha={p.ha} onHa={p.onHa} />}
    </th>
  )
}

function LockHint({ read: { locks, unread } }: { read: LocksRead }) {
  if (unread) return <p class="lockhint">{LOCKS_UNREAD}</p>
  const text = lockHint(locks)
  if (!text) return null
  const entries = [...locks.entries()].sort(([a], [b]) => a - b)
  return (
    <p class="lockhint">
      {text}{' '}
      {entries.map(([g, lock], i) => (
        <span key={g}>
          {i > 0 && ' · '}
          <a class="chref" href={lockHref(lock)}>
            Open CH{lock.channel}
          </a>
        </span>
      ))}
    </p>
  )
}

function OpenHead(p: {
  g: number
  grp: GroupMembershipMatrix['groups'][number]
  count: number
  open: boolean
  ha: boolean
  onOpen: () => void
  onName: (name: string) => void
  onClose: () => void
  onHa: () => void
}) {
  return (
    <th class={`gname${p.grp.dirty ? ' dirty' : ''}`} title="Click to rename" onClick={p.onOpen}>
      G{p.g}
      <span class="cnt">
        {p.grp.name || '—'} · {p.count}
      </span>
      {p.open && (
        <span class="pop" onClick={(e) => e.stopPropagation()}>
          <span class="ph">Group G{p.g}</span>
          <span class="pb">
            <EditableName
              autoFocus
              value={p.grp.name ?? ''}
              placeholder={`unnamed`}
              onCommit={p.onName}
              onCancel={p.onClose}
              onDismiss={p.onClose}
            />
            <span class="prow" onMouseDown={(e) => e.preventDefault()}>
              <span class="pl">Home Assistant</span>
              <Switch on={p.ha} onToggle={p.onHa} />
            </span>
          </span>
        </span>
      )}
    </th>
  )
}

export function Groups() {
  const { data, reload } = useLive(async () => {
    const [matrix, lamps, groups] = await Promise.all([
      api.groupMatrix(ADAPTER),
      api.virtualLamps(ADAPTER),
      api.groups(ADAPTER),
    ])
    const bound = boundLamps(lamps.virtual_lamps)
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
  const locksRead = useGroupLocks()
  const locks = locksRead.locks
  useEffect(() => setEdits((prev) => dropLocked(prev, locks)), [locks])

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

  const serverDirty = matrix.rows.filter(gearDiffers).length
  const gearRows = matrix.rows.map((r) => ({
    virtual_lamp_id: r.virtual_lamp_id,
    differs: gearDiffers(r),
  }))
  const bar = applyBar(edits.size, gearRows, bound)
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
      onApplied: async () => {
        await reload()
        setEdits(new Map())
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
              {matrix.groups.map((grp, g) => {
                const head = {
                  g,
                  count: desiredCount(g),
                  open: renaming === g,
                  ha: haEnabled.get(g) !== false,
                  onHa: () => void toggleGroupHa(g, haEnabled.get(g) === false),
                }
                const lock = locks.get(g)
                return lock ? (
                  <LockedHead key={g} {...head} lock={lock} onOpen={() => setRenaming((cur) => (cur === g ? null : g))} />
                ) : (
                  <OpenHead
                    key={g}
                    {...head}
                    grp={grp}
                    onOpen={() => setRenaming((cur) => (cur === null || locks.has(cur) ? g : cur))}
                    onName={(v) => void saveGroupName(g, v)}
                    onClose={() => setRenaming(null)}
                  />
                )
              })}
            </tr>
          </thead>
          <tbody>
            {visibleRows.map((row) => {
              const vl = row.virtual_lamp_id
              const isBound = bound.has(vl)
              const name = names.get(vl) || row.name
              const rowDirty =
                gearDiffers(row) || [...edits.keys()].some((k) => k.startsWith(`${vl}:`))
              return (
                <tr key={vl} class={rowDirty ? 'rowdirty' : undefined}>
                  <td class="lamp">
                    <span class="vl">VL {pad2(vl)}</span>
                    <span class={`nm${name ? '' : ' name-faint'}`}>{name || '— unnamed'}</span>
                    {!isBound && <span class="unbound">unbound</span>}
                    {rowDirty && isBound && (
                      <button
                        class="act"
                        title="Re-read the gear's group membership — heals a corrupted readback"
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
                        locked={locks.has(g)}
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
      <LockHint read={locksRead} />

      {bar && (
        <div class="applybar">
          {bar.kind === 'edits' ? (
            <span class="txt">
              <b>
                {bar.count} pending edit{bar.count === 1 ? '' : 's'}
              </b>{' '}
              — {localAdd} add · {localRm} remove
            </span>
          ) : (
            <span class="txt">
              <b>
                {bar.count} row{bar.count === 1 ? '' : 's'}
              </b>{' '}
              {bar.count === 1 ? 'differs' : 'differ'} from the gear
            </span>
          )}
          {bar.kind === 'edits' && (
            <button class="btn discard" onClick={discard} disabled={busy}>
              Discard
            </button>
          )}
          <button class="btn apply" onClick={applyToBus} disabled={busy}>
            Apply to bus
          </button>
        </div>
      )}
    </>
  )
}
