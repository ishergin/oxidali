import type {
  ClusterBirthMark,
  ClusterInfo,
  ClusterNodePart,
  ClusterPart,
  ClusterPartWrite,
  ClusterRecord,
  ClusterRecordKind,
} from '../api/types.js'
import { ADAPTER } from '../format.js'
import { MAX_CLUSTER_NAME_BYTES, type Option, clipName, utf8Bytes } from './cluster-view.js'

export type Kind = ClusterRecordKind

export const NUMBERS_PER_NODE = 16
export const RECORD_NUMBER_MAX = 127
export const LEVEL_TOP = 254

const PREFIX: Record<Kind, string> = { channel: 'CH', scene: 'CS' }
const LOCAL_PREFIX: Record<Kind, string> = { channel: 'G', scene: 'S' }
const NOUN: Record<Kind, string> = { channel: 'channel', scene: 'scene' }

export const NEW_LINK = { send: true, accept: false }

export const EMPTY_NAME = 'A name cannot be empty: type one, or Discard to keep the current name.'

export function recordLabel(kind: Kind, number: number): string {
  return `${PREFIX[kind]}${number}`
}

export function localLabel(kind: Kind, local: number, adapter = ADAPTER): string {
  const head = adapter === ADAPTER ? '' : `A${adapter}·`
  return `${head}${LOCAL_PREFIX[kind]}${local}`
}

export function homeOf(number: number): number {
  return Math.floor(number / NUMBERS_PER_NODE)
}

export function localOf(part: ClusterPart): number {
  return part.group ?? part.scene ?? 0
}

export function sameMark(a: ClusterBirthMark, b: ClusterBirthMark): boolean {
  return a.generation === b.generation && a.nonce === b.nonce
}

interface Marked {
  number: number
  birth_mark: ClusterBirthMark
}

export function recordKey(kind: Kind, rec: Marked): string {
  return `${kind}:${rec.number}:${rec.birth_mark.generation}:${rec.birth_mark.nonce}`
}

export function isHome(rec: ClusterRecord): boolean {
  return rec.source === 'own'
}

export interface HomeName {
  editable: boolean
  text: string
  openNode: number | null
}

export function homeName(kind: Kind, rec: ClusterRecord): HomeName {
  const label = recordLabel(kind, rec.number)
  const k = rec.home_node
  if (isHome(rec)) return { editable: true, text: `kept on this node, the home of ${label}`, openNode: null }
  if (rec.source === 'stale')
    return { editable: false, text: `kept on node ${k} · cannot be renamed or deleted until node ${k} answers`, openNode: null }
  return { editable: false, text: `kept on node ${k}, the home of ${label}: renamed and deleted there`, openNode: k }
}

export function homeCell(rec: ClusterRecord): string {
  return isHome(rec) ? 'this node' : `node ${rec.home_node}`
}

export function deleteKey(kind: Kind, rec: Marked): string {
  return `del:${recordKey(kind, rec)}`
}

export function finishKey(kind: Kind, part: ClusterPart): string {
  return `fin:${kind}:${part.adapter}:${localOf(part)}`
}

export interface PartState {
  local: number | null
  send: boolean
  accept: boolean
}

export interface NameEdit {
  name: string
  revision: number
}

export interface Draft {
  names: Record<string, NameEdit>
  parts: Record<string, PartState>
}

export const EMPTY_DRAFT: Draft = { names: {}, parts: {} }

export interface Records {
  channel: ClusterRecord[]
  scene: ClusterRecord[]
}

export interface OwnParts {
  channel: ClusterNodePart[]
  scene: ClusterNodePart[]
}

export function ownPartOf(rec: Marked, own: ClusterNodePart[]): ClusterNodePart | null {
  return (
    own.find(
      (p) => p.adapter === ADAPTER && p.record.number === rec.number && sameMark(p.record.birth_mark, rec.birth_mark),
    ) ?? null
  )
}

export function serverPart(part: ClusterNodePart | null): PartState {
  return part ? { local: localOf(part), send: part.send, accept: part.accept } : { local: null, ...NEW_LINK }
}

export function samePart(a: PartState, b: PartState): boolean {
  if (a.local !== b.local) return false
  return a.local === null || (a.send === b.send && a.accept === b.accept)
}

export function partShown(draft: Draft, kind: Kind, rec: ClusterRecord, own: ClusterNodePart[]): PartState {
  return draft.parts[recordKey(kind, rec)] ?? serverPart(ownPartOf(rec, own))
}

export function nameShown(draft: Draft, kind: Kind, rec: ClusterRecord): string {
  return draft.names[recordKey(kind, rec)]?.name ?? rec.name
}

export function editPart(
  draft: Draft,
  kind: Kind,
  rec: ClusterRecord,
  own: ClusterNodePart[],
  change: Partial<PartState>,
): Draft {
  const key = recordKey(kind, rec)
  const server = serverPart(ownPartOf(rec, own))
  const next = { ...(draft.parts[key] ?? server), ...change }
  const parts = { ...draft.parts }
  if (samePart(next, server)) delete parts[key]
  else parts[key] = next
  return { ...draft, parts }
}

function clipTyped(raw: string): string {
  const points = Array.from(raw)
  while (utf8Bytes(points.join('')) > MAX_CLUSTER_NAME_BYTES) points.pop()
  return points.join('')
}

export function editName(draft: Draft, kind: Kind, rec: ClusterRecord, raw: string): Draft {
  const key = recordKey(kind, rec)
  const name = clipTyped(raw)
  const names = { ...draft.names }
  if (clipName(name) === rec.name) delete names[key]
  else names[key] = { name, revision: draft.names[key]?.revision ?? rec.revision }
  return { ...draft, names }
}

export function dropName(draft: Draft, key: string): Draft {
  if (!(key in draft.names)) return draft
  const names = { ...draft.names }
  delete names[key]
  return { ...draft, names }
}

function recordByKey(records: Records): Map<string, ClusterRecord> {
  const out = new Map<string, ClusterRecord>()
  for (const kind of ['channel', 'scene'] as const) {
    for (const rec of records[kind]) out.set(recordKey(kind, rec), rec)
  }
  return out
}

function kindOfKey(key: string): Kind {
  return key.startsWith('scene:') ? 'scene' : 'channel'
}

export function draftOnRead(draft: Draft, records: Records, own: OwnParts, readOnly = false): Draft {
  if (readOnly) return EMPTY_DRAFT
  const byKey = recordByKey(records)
  const names = Object.entries(draft.names).filter(([key, edit]) => byKey.has(key) && byKey.get(key)?.name !== clipName(edit.name))
  const parts = Object.entries(draft.parts).filter(([key, state]) => {
    const rec = byKey.get(key)
    return rec !== undefined && !samePart(state, serverPart(ownPartOf(rec, own[kindOfKey(key)])))
  })
  const kept = names.length === Object.keys(draft.names).length && parts.length === Object.keys(draft.parts).length
  return kept ? draft : { names: Object.fromEntries(names), parts: Object.fromEntries(parts) }
}

export interface DraftItem {
  key: string
  kind: Kind
  number: number
  what: 'name' | 'part'
  label: string
}

function itemOf(key: string, what: 'name' | 'part'): DraftItem {
  const kind = kindOfKey(key)
  const number = Number(key.split(':')[1])
  return { key, kind, number, what, label: `${recordLabel(kind, number)} ${what}` }
}

const KIND_ORDER: Record<Kind, number> = { channel: 0, scene: 1 }

export function draftItems(draft: Draft): DraftItem[] {
  const items = [
    ...Object.keys(draft.names).map((key) => itemOf(key, 'name')),
    ...Object.keys(draft.parts).map((key) => itemOf(key, 'part')),
  ]
  return items.sort(
    (a, b) =>
      KIND_ORDER[a.kind] - KIND_ORDER[b.kind] || a.number - b.number || (a.what === 'name' ? -1 : 1) - (b.what === 'name' ? -1 : 1),
  )
}

export function draftLine(items: DraftItem[]): string {
  if (items.length === 0) return 'No unsaved changes'
  const noun = items.length === 1 ? 'unsaved change' : 'unsaved changes'
  return `${items.length} ${noun} · ${items.map((i) => i.label).join(', ')}`
}

export function editsOf(draft: Draft, key: string): number {
  return (key in draft.names ? 1 : 0) + (key in draft.parts ? 1 : 0)
}

function stillHolds(kind: Kind, part: ClusterNodePart, draft: Draft): boolean {
  if (part.link === 'orphan') return true
  const moved = draft.parts[recordKey(kind, part.record)]
  return moved === undefined || moved.local === localOf(part)
}

export function holderOf(
  kind: Kind,
  local: number,
  own: ClusterNodePart[],
  draft: Draft,
  exceptKey: string,
): string | null {
  const server = own.find((p) => p.adapter === ADAPTER && localOf(p) === local && stillHolds(kind, p, draft))
  if (server) {
    if (recordKey(kind, server.record) === exceptKey) return null
    const label = recordLabel(kind, server.record.number)
    return server.link === 'orphan' ? `${label} · left over` : label
  }
  for (const [key, state] of Object.entries(draft.parts)) {
    if (key !== exceptKey && kindOfKey(key) === kind && state.local === local) return recordLabel(kind, Number(key.split(':')[1]))
  }
  return null
}

export function takenText(kind: Kind, local: number, holder: string): string {
  return `${localLabel(kind, local)} is the part of ${holder}. Unlink it there first; the draft does not move it.`
}

export interface LocalInfo {
  id: number
  name: string
  members: number
}

export function localOptions(
  kind: Kind,
  locals: LocalInfo[],
  own: ClusterNodePart[],
  draft: Draft,
  key: string,
): Option[] {
  const options: Option[] = [{ value: null, label: 'Not linked' }]
  for (const l of locals) {
    const holder = holderOf(kind, l.id, own, draft, key)
    const name = l.name ? ` · ${l.name}` : ''
    const tail = holder ? ` · part of ${holder}` : ` · ${l.members} lamp${l.members === 1 ? '' : 's'}`
    options.push({ value: l.id, label: `${localLabel(kind, l.id)}${name}${tail}` })
  }
  return options
}

export type Step =
  | { op: 'rename'; key: string; item: string; kind: Kind; number: number; name: string; revision: number }
  | { op: 'unlink'; key: string; item: string; kind: Kind; local: number }
  | { op: 'link'; key: string; item: string; kind: Kind; local: number; body: ClusterPartWrite }
  | { op: 'refuse'; key: string; item: string; kind: Kind; text: string }

function partSteps(kind: Kind, rec: ClusterRecord, state: PartState, own: OwnParts, draft: Draft): Step[] {
  const key = recordKey(kind, rec)
  const item = `${recordLabel(kind, rec.number)} part`
  const server = ownPartOf(rec, own[kind])
  const holder = state.local === null ? null : holderOf(kind, state.local, own[kind], draft, key)
  if (state.local !== null && holder) return [{ op: 'refuse', key, item, kind, text: takenText(kind, state.local, holder) }]
  const steps: Step[] = []
  const was = server ? localOf(server) : null
  if (was !== null && state.local !== was) steps.push({ op: 'unlink', key, item, kind, local: was })
  if (state.local !== null && !samePart(state, serverPart(server))) {
    const record = { number: rec.number, birth_mark: rec.birth_mark }
    steps.push({ op: 'link', key, item, kind, local: state.local, body: { record, send: state.send, accept: state.accept } })
  }
  return steps
}

const STEP_ORDER: Record<Step['op'], number> = { refuse: 0, rename: 1, unlink: 2, link: 3 }

export function planApply(draft: Draft, records: Records, own: OwnParts): Step[] {
  const byKey = recordByKey(records)
  const steps: Step[] = []
  for (const [key, edit] of Object.entries(draft.names)) {
    const rec = byKey.get(key)
    if (!rec) continue
    const kind = kindOfKey(key)
    const item = `${recordLabel(kind, rec.number)} name`
    const name = clipName(edit.name)
    if (name === '') steps.push({ op: 'refuse', key, item, kind, text: EMPTY_NAME })
    else steps.push({ op: 'rename', key, item, kind, number: rec.number, name, revision: edit.revision })
  }
  for (const [key, state] of Object.entries(draft.parts)) {
    const rec = byKey.get(key)
    if (rec) steps.push(...partSteps(kindOfKey(key), rec, state, own, draft))
  }
  return steps.sort((a, b) => STEP_ORDER[a.op] - STEP_ORDER[b.op])
}

export function stepText(kind: Kind, step: Step): string {
  switch (step.op) {
    case 'rename':
      return `renamed to “${step.name}”`
    case 'unlink':
      return `${localLabel(kind, step.local)} unlinked`
    case 'link':
      return `${localLabel(kind, step.local)} linked, Send ${step.body.send ? 'on' : 'off'}, Accept ${step.body.accept ? 'on' : 'off'}`
    case 'refuse':
      return step.text
  }
}

export interface Outcome {
  key: string
  item: string
  what: 'name' | 'part'
  ok: boolean
  text: string
  stale: boolean
}

export interface ApplyPorts {
  rename: (kind: Kind, number: number, name: string, revision: number) => Promise<unknown>
  unlink: (kind: Kind, local: number) => Promise<unknown>
  link: (kind: Kind, local: number, body: ClusterPartWrite) => Promise<unknown>
  refusal: (e: unknown) => string
}

function sendStep(step: Step, ports: ApplyPorts): Promise<unknown> {
  switch (step.op) {
    case 'rename':
      return ports.rename(step.kind, step.number, step.name, step.revision)
    case 'unlink':
      return ports.unlink(step.kind, step.local)
    case 'link':
      return ports.link(step.kind, step.local, step.body)
    case 'refuse':
      return Promise.reject(new Error(step.text))
  }
}

function heldBack(step: Step, stuck: Set<string>): string | null {
  if (step.op !== 'link') return null
  const label = localLabel(step.kind, step.local)
  if (stuck.has(step.key)) return `${label} was not linked: the record's old part could not be removed.`
  if (stuck.has(`${step.kind}:${step.local}`)) return `${label} was not linked: it is still the part of another record.`
  return null
}

export async function runApply(steps: Step[], ports: ApplyPorts): Promise<Outcome[]> {
  const out: Outcome[] = []
  const stuck = new Set<string>()
  for (const step of steps) {
    const what = step.item.endsWith(' name') ? 'name' : 'part'
    const base = { key: step.key, item: step.item, what, stale: false } as const
    const held = step.op === 'refuse' ? step.text : heldBack(step, stuck)
    if (held !== null) {
      out.push({ ...base, ok: false, text: held })
      continue
    }
    try {
      await sendStep(step, ports)
      out.push({ ...base, ok: true, text: stepText(step.kind, step) })
    } catch (e) {
      if (step.op === 'unlink') stuck.add(step.key).add(`${step.kind}:${step.local}`)
      out.push({ ...base, ok: false, text: ports.refusal(e), stale: isConflict(e, 'cluster_stale_revision') })
    }
  }
  return out
}

export function draftAfter(draft: Draft, outcomes: Outcome[]): Draft {
  const failed = new Set(outcomes.filter((o) => !o.ok).map((o) => `${o.what}:${o.key}`))
  const stale = new Set(outcomes.filter((o) => o.stale).map((o) => o.key))
  const keep = (what: string) => ([key]: [string, unknown]) => failed.has(`${what}:${key}`) && !(what === 'name' && stale.has(key))
  return {
    names: Object.fromEntries(Object.entries(draft.names).filter(keep('name'))),
    parts: Object.fromEntries(Object.entries(draft.parts).filter(keep('part'))),
  }
}

export interface FieldErrors {
  name?: string
  part?: string
}

export function fieldErrors(outcomes: Outcome[]): Record<string, FieldErrors> {
  const out: Record<string, FieldErrors> = {}
  for (const o of outcomes) {
    if (!o.ok) out[o.key] = { ...out[o.key], [o.what]: out[o.key]?.[o.what] ?? o.text }
  }
  return out
}

export interface NodeInPart {
  node: number
  silent: boolean
}

export function nodesTakingPart(number: number, parts: ClusterNodePart[]): NodeInPart[] {
  const seen = new Map<number, NodeInPart>()
  for (const p of parts) {
    if (p.record.number !== number || p.link !== 'live' || seen.has(p.node)) continue
    seen.set(p.node, { node: p.node, silent: p.source === 'stale' })
  }
  return [...seen.values()].sort((a, b) => a.node - b.node)
}

export function ignoringNodes(number: number, parts: ClusterNodePart[], ownNode: number): number[] {
  const nodes = parts
    .filter((p) => p.record.number === number && p.link === 'live' && !p.accept && p.node !== ownNode)
    .map((p) => p.node)
  return [...new Set(nodes)].sort((a, b) => a - b)
}

export function recallsOn(kind: Kind, number: number, parts: ClusterNodePart[]): string {
  const live = parts.filter((p) => p.record.number === number && p.link === 'live')
  live.sort((a, b) => a.node - b.node || a.adapter - b.adapter)
  return live.map((p) => `N${p.node} ${localLabel(kind, localOf(p), p.adapter)}`).join(' · ')
}

export function partNodes(info: ClusterInfo | null, kind: Kind, ownNode: number): number[] {
  const nodes = new Set([ownNode])
  for (const row of info?.nodes ?? []) {
    if ((kind === 'channel' ? row.channel_parts : row.scene_parts) > 0) nodes.add(row.node)
  }
  return [...nodes].sort((a, b) => a - b)
}

export function orphansOf(own: ClusterNodePart[]): ClusterNodePart[] {
  return own.filter((p) => p.link === 'orphan').sort((a, b) => a.record.number - b.record.number)
}

export function orphanName(kind: Kind, part: ClusterNodePart, records: ClusterRecord[]): string {
  const again = records.some((r) => r.number === part.record.number)
  return again ? `an earlier ${recordLabel(kind, part.record.number)}` : `deleted ${NOUN[kind]}`
}

export type NumberRead = { ok: true; number: number | null } | { ok: false; text: string; node: number | null }

export function parseRecordNumber(kind: Kind, raw: string, ownNode: number): NumberRead {
  const text = raw.trim().replace(/^(CH|CS)/i, '')
  if (text === '') return { ok: true, number: null }
  const n = /^\d+$/.test(text) ? Number(text) : NaN
  if (!Number.isInteger(n) || n > RECORD_NUMBER_MAX) return { ok: false, text: 'A number is 0–127.', node: null }
  const home = homeOf(n)
  if (home !== ownNode) {
    const label = recordLabel(kind, n)
    return { ok: false, text: `${label} is in the block of node ${home}: create it on node ${home}.`, node: home }
  }
  return { ok: true, number: n }
}

export function blockText(kind: Kind, ownNode: number): string {
  const first = ownNode * NUMBERS_PER_NODE
  return `this node's numbers: ${recordLabel(kind, first)}–${recordLabel(kind, first + NUMBERS_PER_NODE - 1)}`
}

export type Preset = 'own' | 'same'

export interface PlanRow {
  local: number
  label: string
  action: 'create' | 'link' | 'skip'
  number: number
  name: string
  text: string
}

function planRow(kind: Kind, preset: Preset, l: LocalInfo, records: ClusterRecord[], own: ClusterNodePart[], ownNode: number): PlanRow {
  const number = preset === 'own' ? ownNode * NUMBERS_PER_NODE + l.id : l.id
  const base = { local: l.id, label: `${localLabel(kind, l.id)}${l.name ? ` · ${l.name}` : ''}`, number, name: l.name }
  const rec = records.find((r) => r.number === number)
  const target = recordLabel(kind, number)
  const local = localLabel(kind, l.id)
  const holder = own.find((p) => p.adapter === ADAPTER && localOf(p) === l.id)
  if (holder) {
    const tail = holder.link === 'orphan' ? ' · left over' : ''
    return { ...base, action: 'skip', text: `skipped: ${local} is the part of ${recordLabel(kind, holder.record.number)}${tail}` }
  }
  if (rec) {
    const has = ownPartOf(rec, own)
    if (has) return { ...base, action: 'skip', text: `skipped: ${target} already has ${localLabel(kind, localOf(has))} of this node` }
    return { ...base, action: 'link', text: `${target} exists: link ${local} to it` }
  }
  if (homeOf(number) === ownNode) {
    const named = l.name ? ` “${l.name}”` : ''
    return { ...base, action: 'create', text: `create ${target}${named} and link ${local}` }
  }
  const node = homeOf(number)
  return { ...base, action: 'skip', text: `${target} does not exist; only node ${node} creates ${NOUN[kind]}s ${node * NUMBERS_PER_NODE}–${node * NUMBERS_PER_NODE + NUMBERS_PER_NODE - 1}` }
}

export function presetPlan(
  kind: Kind,
  preset: Preset,
  locals: LocalInfo[],
  records: ClusterRecord[],
  own: ClusterNodePart[],
  ownNode: number,
): PlanRow[] {
  return locals.map((l) => planRow(kind, preset, l, records, own, ownNode))
}

export function offeredLocals(locals: LocalInfo[]): LocalInfo[] {
  return locals.filter((l) => l.name !== '' || l.members > 0)
}

export interface PresetPorts {
  create: (kind: Kind, number: number, name: string) => Promise<ClusterRecord>
  link: (kind: Kind, local: number, body: ClusterPartWrite) => Promise<unknown>
  refusal: (e: unknown) => string
}

export interface PresetOutcome {
  local: number
  state: 'done' | 'skipped' | 'refused'
  text: string
}

export async function retryNonce<T>(create: () => Promise<T>): Promise<T> {
  try {
    return await create()
  } catch (e) {
    if (!isNonceTaken(e)) throw e
    return create()
  }
}

function createOnce(kind: Kind, row: PlanRow, ports: PresetPorts): Promise<ClusterRecord> {
  return retryNonce(() => ports.create(kind, row.number, row.name))
}

export function isConflict(e: unknown, cause: string): boolean {
  return e instanceof Error && 'code' in e && (e as { code: unknown }).code === 'conflict' && e.message === cause
}

export function isNonceTaken(e: unknown): boolean {
  return isConflict(e, 'cluster_nonce_taken')
}

async function runRow(kind: Kind, row: PlanRow, records: ClusterRecord[], ports: PresetPorts): Promise<PresetOutcome> {
  if (row.action === 'skip') return { local: row.local, state: 'skipped', text: row.text }
  const rec = row.action === 'create' ? await createOnce(kind, row, ports) : records.find((r) => r.number === row.number)
  if (!rec) return { local: row.local, state: 'refused', text: `${recordLabel(kind, row.number)} is gone` }
  const record = { number: rec.number, birth_mark: rec.birth_mark }
  await ports.link(kind, row.local, { record, ...NEW_LINK })
  const made = row.action === 'create' ? `created ${recordLabel(kind, rec.number)} · ` : ''
  return { local: row.local, state: 'done', text: `${made}${localLabel(kind, row.local)} linked to ${recordLabel(kind, rec.number)}` }
}

export async function runPreset(
  kind: Kind,
  rows: PlanRow[],
  records: ClusterRecord[],
  ports: PresetPorts,
): Promise<PresetOutcome[]> {
  const out: PresetOutcome[] = []
  for (const row of rows) {
    try {
      out.push(await runRow(kind, row, records, ports))
    } catch (e) {
      out.push({ local: row.local, state: 'refused', text: ports.refusal(e) })
    }
  }
  return out
}

export function levelBody(level: number): { power: 'on' | 'off'; level?: number } {
  return level === 0 ? { power: 'off' } : { power: 'on', level: Math.min(LEVEL_TOP, level) }
}

const REACHES: Record<Kind, string> = {
  channel: 'On, Off and the level given on this page still reach',
  scene: 'Recall given on this page still reaches',
}

export function partNote(kind: Kind, number: number, state: PartState): string | null {
  if (state.local === null) return null
  const label = recordLabel(kind, number)
  const local = localLabel(kind, state.local)
  const accept = state.accept
    ? ''
    : `Accept is off: this node will ignore commands for ${label} from other nodes. ${REACHES[kind]} ${local}. `
  const kept =
    kind === 'channel'
      ? `${local} is linked as it is and keeps its name and lamps; they cannot be changed from this UI until the channel editor exists.`
      : ''
  const text = `${accept}${kept}`.trim()
  return text === '' ? null : text
}

export function ignoreNote(kind: Kind, number: number, node: number): string {
  const tail = kind === 'channel' ? 'so nothing sent from here changes its lamps' : 'so a Recall from here does not reach it'
  return `Node ${node} has Accept off: it will ignore commands for ${recordLabel(kind, number)}, ${tail}.`
}

export function presetUnit(kind: Kind, preset: Preset, ownNode: number): string {
  const local = kind === 'channel' ? 'group N' : 'scene N'
  const target = kind === 'channel' ? 'channel' : 'cluster scene'
  if (preset === 'own') return `${local} becomes ${target} 16·node + N of this node, created here`
  if (ownNode === 0) return `${local} goes to ${target} N; the missing ones are created here`
  return `${local} goes to ${target} N; on node ${ownNode} only existing ${NOUN[kind]}s are linked`
}

export function ignoresChip(nodes: number[]): string | null {
  if (nodes.length === 0) return null
  if (nodes.length === 1) return `node ${nodes[0]} ignores`
  return `nodes ${nodes.join(', ')} ignore`
}
