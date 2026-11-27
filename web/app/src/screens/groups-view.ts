import type { ClusterNodePart, ClusterPartLink, ClusterRecord } from '../api/types.js'

export interface MembershipRow {
  desired: boolean[]
  applied: boolean[]
}

export function gearDiffers(row: MembershipRow): boolean {
  return row.desired.some((wanted, group) => wanted !== row.applied[group])
}

export interface GroupLock {
  channel: number
  name: string
  link: ClusterPartLink
}

export function groupLocks(
  adapter: number,
  parts: Pick<ClusterNodePart, 'adapter' | 'group' | 'record' | 'link'>[],
  records: Pick<ClusterRecord, 'number' | 'name'>[],
): Map<number, GroupLock> {
  const out = new Map<number, GroupLock>()
  for (const p of parts) {
    if (p.adapter !== adapter || p.group === undefined) continue
    const rec = p.link === 'live' ? records.find((r) => r.number === p.record.number) : undefined
    out.set(p.group, { channel: p.record.number, name: rec?.name ?? '', link: p.link })
  }
  return out
}

const LOCK_TAIL: Record<ClusterPartLink, string> = { live: '', orphan: ' · left over', unknown: ' · not heard' }

export function lockLabel(lock: GroupLock): string {
  if (lock.link !== 'live') return `CH${lock.channel}${LOCK_TAIL[lock.link]}`
  return lock.name ? `CH${lock.channel} · ${lock.name}` : `CH${lock.channel}`
}

export function lockHref(lock: GroupLock): string {
  return lock.link === 'live' ? `#/cluster/channels/${lock.channel}` : '#/cluster'
}

export const LOCKS_UNREAD =
  'The cluster’s channel parts could not be read, so a column whose group is linked to a cluster channel is not marked here.'

export function lockHint(locks: Map<number, GroupLock>): string | null {
  const entries = [...locks.entries()].sort(([a], [b]) => a - b)
  if (entries.length === 0) return null
  const named = entries.map(([g, lock], i) => (i === 0 ? `G${g} is linked to cluster channel ${lockLabel(lock)}` : `G${g} to ${lockLabel(lock)}`))
  const list = named.length === 1 ? named[0] : `${named.slice(0, -1).join(', ')} and ${named[named.length - 1]}`
  const cols = entries.length === 1 ? 'its column is' : 'their columns are'
  const heads = entries.length === 1 ? 'its header opens' : 'their header opens'
  const lamps = entries.length === 1 ? 'its lamps' : 'their lamps'
  return `${list}, and a group linked to a channel is edited in the channel: ${cols} closed here, and a click on ${heads} the channel's link and the Home Assistant switch, not a name field. The channel's lamp editor is not built yet, so ${lamps} cannot be changed from this UI until it is; to change them here, unlink the group in its channel first.`
}

export function dropLocked(edits: Map<string, boolean>, locks: Map<number, GroupLock>): Map<string, boolean> {
  const kept = [...edits.entries()].filter(([key]) => !locks.has(Number(key.split(':')[1])))
  return kept.length === edits.size ? edits : new Map(kept)
}
