import { Fragment, type ComponentChildren } from 'preact'
import { useEffect, useState } from 'preact/hooks'
import { ApiError, api } from '../api/client'
import type {
  ClusterInfo,
  ClusterNode,
  ClusterNodePart,
  ClusterRecord,
  ClusterSettings,
  ClusterStatus,
} from '../api/types'
import { statusOnly, useClusterStatus } from '../cluster-feed'
import { Chip, SelChip, Switch, useConfirmTap } from '../components/ui'
import { ADAPTER } from '../format'
import { useLive } from '../hooks'
import { notify } from '../toast'
import {
  type ApplyPorts,
  type Draft,
  EMPTY_DRAFT,
  type FieldErrors,
  type Kind,
  LEVEL_TOP,
  type LocalInfo,
  type Outcome,
  type OwnParts,
  type PartState,
  type PlanRow,
  type Preset,
  type PresetOutcome,
  type Records,
  blockText,
  deleteKey,
  finishKey,
  homeName,
  homeCell,
  draftAfter,
  draftItems,
  draftLine,
  draftOnRead,
  editName,
  editPart,
  editsOf,
  fieldErrors,
  holderOf,
  ignoreNote,
  ignoresChip,
  ignoringNodes,
  levelBody,
  localLabel,
  localOptions,
  localOf,
  nameShown,
  nodesTakingPart,
  offeredLocals,
  orphanName,
  orphansOf,
  strayFace,
  parseRecordNumber,
  partNodes,
  partNote,
  partShown,
  planApply,
  presetPlan,
  presetUnit,
  recallsOn,
  recordKey,
  recordLabel,
  retryNonce,
  runApply,
  runPreset,
  sameMark,
  takenText,
  unreadText,
} from './cluster-records'
import { Select, StandbyBanner, refusalFrom, usePeerUrl } from './cluster-shared'
import { ageText, nodeHref, nodeLine } from './cluster-view'

const SETTINGS_HREF = '#/settings/cluster'
const SETTINGS_CAUSES = new Set(['cluster_no_address', 'cluster_not_running'])
const TAB_HREF: Record<Kind, string> = { channel: '#/cluster', scene: '#/cluster/scenes' }
const TAB_TITLE: Record<Kind, string> = { channel: 'Channels', scene: 'Scenes' }
const CARD_TITLE: Record<Kind, string> = { channel: 'Channels', scene: 'Cluster scenes' }
const NEW_LABEL: Record<Kind, string> = { channel: 'New channel', scene: 'New scene' }
const LOCAL_NOUN: Record<Kind, string> = { channel: 'group', scene: 'scene' }

interface World {
  settings: ClusterSettings
  info: ClusterInfo | null
  records: Records
  own: OwnParts
  all: OwnParts
  unread: Record<Kind, number[]>
  locals: Record<Kind, LocalInfo[]>
}

interface PartsRead {
  parts: ClusterNodePart[]
  unread: number[]
}

async function readParts(kind: Kind, nodes: number[], me: number): Promise<PartsRead> {
  const other = (n: number) => api.clusterNodeParts(kind, n).catch(() => null)
  const lists = await Promise.all(nodes.map((n) => (n === me ? api.clusterNodeParts(kind, n) : other(n))))
  return {
    parts: lists.flatMap((l) => l ?? []),
    unread: nodes.filter((_, i) => lists[i] === null),
  }
}

async function readLocals(): Promise<Record<Kind, LocalInfo[]>> {
  const [groups, scenes] = await Promise.all([api.groups(ADAPTER), api.scenes(ADAPTER)])
  return {
    channel: groups.groups.map((g) => ({ id: g.group_id, name: g.name, members: g.member_count_desired })),
    scene: scenes.scenes.map((s) => ({ id: s.scene_id, name: s.name, members: s.row_count_included })),
  }
}

async function readWorld(): Promise<World> {
  const [settings, info, channel, scene, locals] = await Promise.all([
    api.clusterSettings(),
    api.clusterInfo().catch(() => null),
    api.clusterRecords('channel'),
    api.clusterRecords('scene'),
    readLocals(),
  ])
  const me = settings.node_number
  const [ch, sc] = await Promise.all([
    readParts('channel', partNodes(info, 'channel', me), me),
    readParts('scene', partNodes(info, 'scene', me), me),
  ])
  const mine = (p: ClusterNodePart) => p.node === me
  const own = { channel: ch.parts.filter(mine), scene: sc.parts.filter(mine) }
  const all = { channel: ch.parts, scene: sc.parts }
  return { settings, info, records: { channel, scene }, own, all, unread: { channel: ch.unread, scene: sc.unread }, locals }
}

interface RowErr {
  text: string
  settings: boolean
}

function rowErrOf(e: unknown): RowErr {
  const settings = e instanceof ApiError && SETTINGS_CAUSES.has(e.message)
  return { text: refusalFrom(e, 'commands').text, settings }
}

interface Ctx {
  world: World
  kind: Kind
  me: number
  readOnly: boolean
  busy: boolean
  status: ClusterStatus | null
  draft: Draft
  edit: (fn: (d: Draft) => Draft, key: string) => void
  errs: Record<string, FieldErrors>
  setErr: (key: string, what: keyof FieldErrors, text: string) => void
  rowErr: Record<string, RowErr>
  command: (rec: ClusterRecord, title: string, send: () => Promise<unknown>) => void
  armed: string | null
  tap: (key: string, act: () => void) => void
  reload: () => void
  isOpen: (n: number) => boolean
  toggle: (n: number) => void
  openRow: (n: number) => void
}

function statusNode(ctx: Ctx, node: number): ClusterNode | null {
  return ctx.status?.nodes.find((n) => n.node === node) ?? null
}

function nodeAge(node: ClusterNode | null): string {
  if (!node) return ''
  return node.heard ? `heard ${ageText(node.age_ms)}` : `silent ${ageText(node.age_ms)}`
}

function OpenNode({ node, label = 'Open' }: { node: ClusterNode | null; label?: string }) {
  if (!node) return null
  return (
    <a class="btn sm ghost" href={nodeHref(node)} target="_blank" rel="noopener noreferrer">
      {label}
    </a>
  )
}

function Unread({ nodes, lead }: { nodes: number[]; lead: boolean }) {
  if (nodes.length === 0) return null
  return (
    <span class="silent" title="these nodes' parts could not be read">
      {lead ? ' · ' : ''}
      {unreadText(nodes)}
    </span>
  )
}

function NodesCell({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const unread = ctx.world.unread[ctx.kind]
  if (ctx.kind === 'scene') {
    const recalls = recallsOn('scene', rec.number, ctx.world.all.scene)
    return (
      <td class="nodes">
        {recalls}
        <Unread nodes={unread} lead={recalls !== ''} />
      </td>
    )
  }
  const nodes = nodesTakingPart(rec.number, ctx.world.all.channel)
  return (
    <td class="nodes">
      {nodes.map((n, i) => (
        <Fragment key={n.node}>
          {i > 0 && ' · '}
          {n.silent ? (
            <span class="silent" title="silent">
              {n.node}
            </span>
          ) : (
            n.node
          )}
        </Fragment>
      ))}
      <Unread nodes={unread} lead={nodes.length > 0} />
    </td>
  )
}

function ChannelControls({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const label = recordLabel('channel', rec.number)
  const send = (title: string, body: ReturnType<typeof levelBody>) =>
    ctx.command(rec, `${label} ${title}`, () => api.clusterChannelTarget(rec.number, body))
  const sendLevel = (el: HTMLInputElement) => {
    const level = Number(el.value)
    el.value = '0'
    send(`level ${level}`, levelBody(level))
  }
  return (
    <span class="btns">
      <button class="btn sm" disabled={ctx.readOnly} onClick={() => send('on', { power: 'on' })}>
        On
      </button>
      <button class="btn sm" disabled={ctx.readOnly} onClick={() => send('off', { power: 'off' })}>
        Off
      </button>
      <input
        class="slider mini blind"
        type="range"
        min="0"
        max={LEVEL_TOP}
        defaultValue="0"
        disabled={ctx.readOnly}
        aria-label={`Level of ${label}`}
        title="Drag and release to send; with the keyboard, move and press Enter"
        onPointerDown={(e) => {
          e.currentTarget.dataset.drag = '1'
        }}
        onChange={(e) => {
          const el = e.currentTarget
          if (el.dataset.drag !== '1') return
          delete el.dataset.drag
          sendLevel(el)
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') sendLevel(e.currentTarget)
        }}
        onBlur={(e) => {
          e.currentTarget.value = '0'
        }}
      />
    </span>
  )
}

function Controls({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const label = recordLabel(ctx.kind, rec.number)
  return (
    <td class="acts-col" onClick={(e) => e.stopPropagation()}>
      {ctx.kind === 'channel' ? (
        <ChannelControls ctx={ctx} rec={rec} />
      ) : (
        <button
          class="btn sm recall"
          disabled={ctx.readOnly}
          onClick={() => ctx.command(rec, `${label} recall`, () => api.recallClusterScene(rec.number))}
        >
          Recall
        </button>
      )}
    </td>
  )
}

function NameChips({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const edits = editsOf(ctx.draft, recordKey(ctx.kind, rec))
  const ignores = ignoresChip(ignoringNodes(rec.number, ctx.world.all[ctx.kind], ctx.me))
  return (
    <>
      {edits > 0 && <Chip cls="warn">{edits === 1 ? '1 edit' : `${edits} edits`}</Chip>}
      {rec.source === 'stale' && <Chip cls="muted">home node {rec.home_node} silent</Chip>}
      {ignores && <Chip cls="muted">{ignores}</Chip>}
    </>
  )
}

function RecordRow({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const label = recordLabel(ctx.kind, rec.number)
  const open = ctx.isOpen(rec.number)
  return (
    <tr class={open ? 'exprow open' : 'exprow'} onClick={() => ctx.toggle(rec.number)}>
      <td class="num">
        <button
          class="chev"
          aria-expanded={open}
          aria-label={`${open ? 'Close' : 'Open'} ${label}`}
          onClick={(e) => {
            e.stopPropagation()
            ctx.toggle(rec.number)
          }}
        />
        {label}
      </td>
      <td class="nm-col">
        <span class="nm">{rec.name}</span>
        <NameChips ctx={ctx} rec={rec} />
      </td>
      <td class="unit">{homeCell(rec)}</td>
      <NodesCell ctx={ctx} rec={rec} />
      <Controls ctx={ctx} rec={rec} />
    </tr>
  )
}

function RowError({ err }: { err: RowErr | undefined }) {
  if (!err) return null
  return (
    <tr class="rowerr">
      <td />
      <td colSpan={4} role="alert">
        Not sent: {err.text}
        {err.settings && (
          <>
            {' '}
            <a href={SETTINGS_HREF}>Cluster settings</a>
          </>
        )}
      </td>
    </tr>
  )
}

function deleteRecord(ctx: Ctx, rec: ClusterRecord) {
  const key = recordKey(ctx.kind, rec)
  const label = recordLabel(ctx.kind, rec.number)
  api
    .deleteClusterRecord(ctx.kind, rec.number, rec.revision)
    .then(() => {
      notify(`Delete ${label}`, 'succeeded', 'Deleted')
      ctx.edit((d) => ({ names: without(d.names, key), parts: without(d.parts, key) }), key)
      ctx.reload()
    })
    .catch((e) => ctx.setErr(key, 'name', refusalFrom(e, 'records').text))
}

function without<T>(map: Record<string, T>, key: string): Record<string, T> {
  const next = { ...map }
  delete next[key]
  return next
}

function NameField({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const key = recordKey(ctx.kind, rec)
  const err = ctx.errs[key]?.name
  const dirty = key in ctx.draft.names
  return (
    <input
      class={err ? 'txt err' : dirty ? 'txt dirty' : 'txt'}
      value={nameShown(ctx.draft, ctx.kind, rec)}
      aria-label={`Name of ${recordLabel(ctx.kind, rec.number)}`}
      disabled={ctx.readOnly || ctx.busy}
      onInput={(e) => {
        const raw = e.currentTarget.value
        ctx.edit((d) => editName(d, ctx.kind, rec, raw), key)
      }}
    />
  )
}

function DeleteButton({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const tapKey = deleteKey(ctx.kind, rec)
  const sure = ctx.armed === tapKey
  return (
    <button
      class={sure ? 'btn sm sure' : 'btn sm'}
      disabled={ctx.readOnly || ctx.busy}
      onClick={() => ctx.tap(tapKey, () => deleteRecord(ctx, rec))}
    >
      {sure ? 'Sure?' : 'Delete'}
    </button>
  )
}

function NameRow({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const home = homeName(ctx.kind, rec)
  const err = ctx.errs[recordKey(ctx.kind, rec)]?.name
  return (
    <div class="mrow">
      <span class="mnode">Name</span>
      {home.editable ? <NameField ctx={ctx} rec={rec} /> : <span class="nm">{rec.name}</span>}
      <span class="unit">{home.text}</span>
      <span class="mgrow" />
      {home.editable && <DeleteButton ctx={ctx} rec={rec} />}
      {home.openNode !== null && (
        <OpenNode node={statusNode(ctx, home.openNode)} label={`Open node ${home.openNode}`} />
      )}
      {err && <span class="fielderr">{err}</span>}
    </div>
  )
}

function withShown(options: { value: number | null; label: string }[], kind: Kind, local: number | null) {
  if (local === null || options.some((o) => o.value === local)) return options
  return [...options, { value: local, label: localLabel(kind, local) }]
}

function PartFlags({ ctx, rec, shown }: { ctx: Ctx; rec: ClusterRecord; shown: PartState }) {
  const key = recordKey(ctx.kind, rec)
  const off = ctx.readOnly || ctx.busy
  const flip = (change: Partial<PartState>) =>
    ctx.edit((d) => editPart(d, ctx.kind, rec, ctx.world.own[ctx.kind], change), key)
  return (
    <>
      <span class="flag">
        Send <Switch on={shown.send} disabled={off} onToggle={() => flip({ send: !shown.send })} />
      </span>
      <span class="flag">
        Accept <Switch on={shown.accept} disabled={off} onToggle={() => flip({ accept: !shown.accept })} />
      </span>
      {ctx.kind === 'scene' && shown.local !== null && (
        <a class="btn sm ghost" href={`#/scenes/${shown.local}`}>
          Edit scene
        </a>
      )}
    </>
  )
}

function OwnPart({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const { kind, world, draft } = ctx
  const key = recordKey(kind, rec)
  const shown = partShown(draft, kind, rec, world.own[kind])
  const holder = shown.local === null ? null : holderOf(kind, shown.local, world.own[kind], draft, key)
  const err = ctx.errs[key]?.part ?? (holder && shown.local !== null ? takenText(kind, shown.local, holder) : null)
  const options = withShown(localOptions(kind, world.locals[kind], world.own[kind], draft, key), kind, shown.local)
  const board = ctx.status?.own.board
  const note = partNote(kind, rec.number, shown)
  return (
    <>
      <div class="mrow">
        <span class="mnode">
          Node {ctx.me} · this node{board && <em>{board}</em>}
        </span>
        <Select
          value={shown.local}
          options={options}
          disabled={ctx.readOnly || ctx.busy}
          bad={err !== null}
          dirty={key in draft.parts}
          label={`This node's ${LOCAL_NOUN[kind]} in ${recordLabel(kind, rec.number)}`}
          onPick={(local) => ctx.edit((d) => editPart(d, kind, rec, world.own[kind], { local }), key)}
        />
        {shown.local !== null && <PartFlags ctx={ctx} rec={rec} shown={shown} />}
        {err && <span class="fielderr">{err}</span>}
      </div>
      {note && (
        <div class="mrow">
          <span class="mnote">{note}</span>
        </div>
      )}
    </>
  )
}

function useRecordParts(ctx: Ctx, rec: ClusterRecord): ClusterNodePart[] {
  const fallback = ctx.world.all[ctx.kind].filter(
    (p) => p.record.number === rec.number && sameMark(p.record.birth_mark, rec.birth_mark),
  )
  const [parts, setParts] = useState<ClusterNodePart[] | null>(null)
  const { kind, world } = ctx
  useEffect(() => {
    let live = true
    api
      .clusterRecord(kind, rec.number)
      .then((d) => {
        if (live) setParts(sameMark(d.birth_mark, rec.birth_mark) ? d.parts : null)
      })
      .catch(() => {
        if (live) setParts(null)
      })
    return () => {
      live = false
    }
  }, [kind, rec.number, rec.birth_mark.generation, rec.birth_mark.nonce, world])
  return parts ?? fallback
}

function NodeHead({ ctx, node }: { ctx: Ctx; node: number }) {
  const sn = statusNode(ctx, node)
  return (
    <span class="mnode">
      Node {node}
      {sn && ` · ${sn.board}`}
      {sn && <em>{nodeAge(sn)}</em>}
    </span>
  )
}

const NOT_IN: Record<Kind, string> = {
  channel: 'Not in this channel. A node links one of its groups on its own page.',
  scene: 'Not in this scene. A node chooses its scene on its own page.',
}

function OtherPart({ ctx, rec, node, part }: { ctx: Ctx; rec: ClusterRecord; node: number; part: ClusterNodePart | null }) {
  const sn = statusNode(ctx, node)
  if (!part)
    return (
      <div class="mrow">
        <NodeHead ctx={ctx} node={node} />
        <span class="mnote">{NOT_IN[ctx.kind]}</span>
        <span class="mgrow" />
        <OpenNode node={sn} />
      </div>
    )
  return (
    <>
      <div class="mrow">
        <NodeHead ctx={ctx} node={node} />
        <span class="mono">{localLabel(ctx.kind, localOf(part), part.adapter)}</span>
        <span class="flag">{part.send ? 'sends' : 'does not send'}</span>
        <span class="flag">{part.accept ? 'accepts' : 'does not accept'}</span>
        {part.source === 'stale' && <Chip cls="muted">as last heard</Chip>}
        <span class="mgrow" />
        <OpenNode node={sn} />
      </div>
      {!part.accept && (
        <div class="mrow">
          <span class="mnote">{ignoreNote(ctx.kind, rec.number, node)}</span>
        </div>
      )}
    </>
  )
}

function OtherParts({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  const parts = useRecordParts(ctx, rec).filter((p) => p.node !== ctx.me && p.link === 'live')
  const nodes = new Set([...parts.map((p) => p.node), ...(ctx.status?.nodes ?? []).map((n) => n.node)])
  nodes.delete(ctx.me)
  return (
    <>
      {[...nodes]
        .sort((a, b) => a - b)
        .map((node) => (
          <OtherPart key={node} ctx={ctx} rec={rec} node={node} part={parts.find((p) => p.node === node) ?? null} />
        ))}
    </>
  )
}

function RecordDetail({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  return (
    <tr class="expdetail">
      <td />
      <td colSpan={4}>
        <div class="members">
          <NameRow ctx={ctx} rec={rec} />
          <OwnPart ctx={ctx} rec={rec} />
          <OtherParts ctx={ctx} rec={rec} />
        </div>
      </td>
    </tr>
  )
}

function finishDelete(ctx: Ctx, part: ClusterNodePart) {
  const local = localOf(part)
  const title = `${strayFace(part).act} of ${localLabel(ctx.kind, local)}`
  api
    .deleteClusterPart(ctx.kind, part.adapter, local)
    .then(() => {
      notify(title, 'succeeded', 'Removed')
      ctx.reload()
    })
    .catch((e) => {
      notify(title, 'failed', refusalFrom(e, 'records').text)
      ctx.reload()
    })
}

function OrphanRow({ ctx, part }: { ctx: Ctx; part: ClusterNodePart }) {
  const local = localOf(part)
  const key = finishKey(ctx.kind, part)
  const sure = ctx.armed === key
  const face = strayFace(part)
  return (
    <tr>
      <td class="num">{recordLabel(ctx.kind, part.record.number)}</td>
      <td class="nm-col">
        <span class="nm name-faint">{orphanName(ctx.kind, part, ctx.world.records[ctx.kind])}</span>
        <Chip cls="muted">{face.chip}</Chip>
      </td>
      <td class="unit">this node</td>
      <td class="nodes">{localLabel(ctx.kind, local, part.adapter)} of this node</td>
      <td class="acts-col">
        <button
          class={sure ? 'btn sm sure' : 'btn sm'}
          disabled={ctx.readOnly || ctx.busy}
          onClick={() => ctx.tap(key, () => finishDelete(ctx, part))}
        >
          {sure ? 'Sure?' : face.act}
        </button>
      </td>
    </tr>
  )
}

function RecordTable({ ctx }: { ctx: Ctx }) {
  const records = ctx.world.records[ctx.kind]
  const orphans = orphansOf(ctx.world.own[ctx.kind])
  const scene = ctx.kind === 'scene'
  return (
    <div class="table-wrap">
      <table class="ctable">
        <thead>
          <tr>
            <th>{scene ? 'Scene' : 'Channel'}</th>
            <th>Name</th>
            <th>Home</th>
            <th>{scene ? 'Recalls on' : 'Nodes'}</th>
            <th class="acts-col">{scene ? '' : 'Control'}</th>
          </tr>
        </thead>
        <tbody>
          {records.map((rec) => (
            <RecordRows key={recordKey(ctx.kind, rec)} ctx={ctx} rec={rec} />
          ))}
          {orphans.map((p) => (
            <OrphanRow key={`${p.adapter}:${localOf(p)}`} ctx={ctx} part={p} />
          ))}
        </tbody>
      </table>
    </div>
  )
}

function RecordRows({ ctx, rec }: { ctx: Ctx; rec: ClusterRecord }) {
  return (
    <>
      <RecordRow ctx={ctx} rec={rec} />
      <RowError err={ctx.rowErr[`${ctx.kind}:${rec.number}`]} />
      {ctx.isOpen(rec.number) && <RecordDetail ctx={ctx} rec={rec} />}
    </>
  )
}

interface NumberErr {
  text: string
  node: number | null
}

function NewRecord({ ctx, onClose }: { ctx: Ctx; onClose: () => void }) {
  const [name, setName] = useState('')
  const [num, setNum] = useState('')
  const [err, setErr] = useState<NumberErr | null>(null)
  const [busy, setBusy] = useState(false)
  const { kind, me } = ctx
  const create = async () => {
    const read = parseRecordNumber(kind, num, me)
    if (!read.ok) return setErr({ text: read.text, node: read.node })
    setBusy(true)
    try {
      const body = { ...(read.number === null ? {} : { number: read.number }), ...(name.trim() ? { name: name.trim() } : {}) }
      const rec = await retryNonce(() => api.createClusterRecord(kind, body))
      notify(`${NEW_LABEL[kind]}`, 'succeeded', `Created ${recordLabel(kind, rec.number)}`)
      ctx.openRow(rec.number)
      ctx.reload()
      onClose()
    } catch (e) {
      setErr({ text: refusalFrom(e, 'records').text, node: null })
    } finally {
      setBusy(false)
    }
  }
  return (
    <NewRecordStrip kind={kind} me={me} name={name} num={num} err={err} busy={busy} ctx={ctx}
      setName={setName} setNum={(v) => { setNum(v); setErr(null) }} create={() => void create()} onClose={onClose} />
  )
}

function NewRecordStrip(p: {
  ctx: Ctx
  kind: Kind
  me: number
  name: string
  num: string
  err: NumberErr | null
  busy: boolean
  setName: (v: string) => void
  setNum: (v: string) => void
  create: () => void
  onClose: () => void
}) {
  const noun = p.kind === 'channel' ? 'channel' : 'scene'
  const node = p.err?.node ?? null
  return (
    <div class="newrec">
      <input class="txt" placeholder={`Name — ${p.kind === 'channel' ? 'Channel' : 'Scene'} N when empty`}
        aria-label={`Name of the new ${noun}`} value={p.name} onInput={(e) => p.setName(e.currentTarget.value)} />
      <input class={p.err ? 'txt num err' : 'txt num'} placeholder="next" aria-label={`Number of the new ${noun}`}
        value={p.num} onInput={(e) => p.setNum(e.currentTarget.value)} />
      <span class="unit">{blockText(p.kind, p.me)}</span>
      <span class="mgrow" />
      <button class="btn sm primary" disabled={p.busy} onClick={p.create}>Create</button>
      <button class="btn sm ghost" onClick={p.onClose}>Close</button>
      {p.err && (
        <span class="fielderr">
          {p.err.text}{' '}
          {node !== null && statusNode(p.ctx, node) && (
            <a href={nodeHref(statusNode(p.ctx, node) as ClusterNode)} target="_blank" rel="noopener noreferrer">
              Open node {node}
            </a>
          )}
        </span>
      )}
    </div>
  )
}

const OUTCOME_CHIP: Record<PresetOutcome['state'], [string, string]> = {
  done: ['ok', 'done'],
  skipped: ['muted', 'skipped'],
  refused: ['err', 'refused'],
}

function PlanLine({ row, ticked, outcome, disabled, onTick }: {
  row: PlanRow
  ticked: boolean
  outcome: PresetOutcome | undefined
  disabled: boolean
  onTick: () => void
}) {
  const chip = outcome ? OUTCOME_CHIP[outcome.state] : null
  return (
    <div class="planrow">
      <input type="checkbox" checked={ticked} disabled={disabled || row.action === 'skip'} aria-label={row.label} onChange={onTick} />
      <span class="mono">{row.label}</span>
      <span class="what">{outcome && outcome.state !== 'skipped' ? outcome.text : row.text}</span>
      {chip ? <Chip cls={chip[0]}>{chip[1]}</Chip> : <span />}
    </div>
  )
}

interface PresetRun {
  rows: PlanRow[]
  outcomes: PresetOutcome[]
}

function usePresetRun(ctx: Ctx) {
  const [run, setRun] = useState<PresetRun | null>(null)
  const [busy, setBusy] = useState(false)
  const start = async (rows: PlanRow[], ticked: (r: PlanRow) => boolean) => {
    setBusy(true)
    const chosen = rows.filter(ticked)
    const outcomes = await runPreset(ctx.kind, chosen, ctx.world.records[ctx.kind], {
      create: (kind, number, name) => api.createClusterRecord(kind, { number, ...(name ? { name } : {}) }),
      link: (kind, local, body) => api.putClusterPart(kind, ADAPTER, local, body),
      refusal: (e) => refusalFrom(e, 'records').text,
    })
    setRun({ rows, outcomes })
    setBusy(false)
    ctx.reload()
  }
  return { run, setRun, busy, start }
}

function Presets({ ctx, onClose }: { ctx: Ctx; onClose: () => void }) {
  const { kind, world, me } = ctx
  const [preset, setPreset] = useState<Preset>(kind === 'channel' ? 'own' : 'same')
  const [unticked, setUnticked] = useState<Set<number>>(new Set())
  const { run, setRun, busy, start } = usePresetRun(ctx)
  const live = presetPlan(kind, preset, offeredLocals(world.locals[kind]), world.records[kind], world.own[kind], me)
  const rows = run?.rows ?? live
  const ticked = (r: PlanRow) => !unticked.has(r.local)
  const todo = rows.filter((r) => r.action !== 'skip' && ticked(r)).length
  const pick = (next: Preset) => {
    setPreset(next)
    setRun(null)
  }
  const tick = (local: number) => {
    const next = new Set(unticked)
    if (!next.delete(local)) next.add(local)
    setUnticked(next)
  }
  return (
    <>
      <div class="newrec">
        {kind === 'channel' && <SelChip on={preset === 'own'} onClick={() => pick('own')}>Own channels</SelChip>}
        <SelChip on={preset === 'same'} onClick={() => pick('same')}>Same numbers</SelChip>
        <span class="unit">{presetUnit(kind, preset, me)}</span>
      </div>
      <PlanBody rows={rows} run={run} busy={busy} ticked={ticked} tick={tick} todo={todo} kind={kind}
        onDo={() => void start(live, ticked)} onClose={onClose} />
    </>
  )
}

function PlanActions({ run, busy, todo, onDo, onClose }: {
  run: PresetRun | null
  busy: boolean
  todo: number
  onDo: () => void
  onClose: () => void
}) {
  return (
    <div class="planrow">
      <span />
      <span />
      <span class="mgrow" />
      <span class="btns">
        {!run && (
          <button class="btn sm primary" disabled={busy || todo === 0} onClick={onDo}>
            Do {todo}
          </button>
        )}
        <button class="btn sm ghost" onClick={onClose}>
          Close
        </button>
      </span>
    </div>
  )
}

function PlanBody(p: {
  kind: Kind
  rows: PlanRow[]
  run: PresetRun | null
  busy: boolean
  ticked: (r: PlanRow) => boolean
  tick: (local: number) => void
  todo: number
  onDo: () => void
  onClose: () => void
}) {
  const outcomeOf = (r: PlanRow) => p.run?.outcomes.find((o) => o.local === r.local)
  return (
    <div class="plan">
      {p.rows.length === 0 && (
        <div class="planrow">
          <span />
          <span class="what">No {LOCAL_NOUN[p.kind]} of this node has a name or members.</span>
        </div>
      )}
      {p.rows.map((r) => (
        <PlanLine key={r.local} row={r} ticked={p.ticked(r)} outcome={outcomeOf(r)} disabled={p.busy || p.run !== null}
          onTick={() => p.tick(r.local)} />
      ))}
      <PlanActions run={p.run} busy={p.busy} todo={p.todo} onDo={p.onDo} onClose={p.onClose} />
    </div>
  )
}

type Strip = 'new' | 'preset' | null

function RecordsCard({ ctx }: { ctx: Ctx }) {
  const [strip, setStrip] = useState<Strip>(null)
  useEffect(() => setStrip(null), [ctx.kind])
  const toggle = (next: Strip) => setStrip(strip === next ? null : next)
  const presetLabel = ctx.kind === 'channel' ? "From this node's groups…" : 'Same numbers'
  const off = ctx.readOnly || ctx.busy
  const empty = ctx.world.records[ctx.kind].length === 0 && orphansOf(ctx.world.own[ctx.kind]).length === 0
  return (
    <div class="card">
      <header>
        <h3>{CARD_TITLE[ctx.kind]}</h3>
        <span class="btns">
          <button class={strip === 'new' ? 'btn sm primary' : 'btn sm'} disabled={off} onClick={() => toggle('new')}>
            {NEW_LABEL[ctx.kind]}
          </button>
          <button class={strip === 'preset' ? 'btn sm primary' : 'btn sm'} disabled={off} onClick={() => toggle('preset')}>
            {presetLabel}
          </button>
        </span>
      </header>
      {strip === 'new' && <NewRecord ctx={ctx} onClose={() => setStrip(null)} />}
      {strip === 'preset' && <Presets ctx={ctx} onClose={() => setStrip(null)} />}
      {empty ? <EmptyRecords kind={ctx.kind} /> : <RecordTable ctx={ctx} />}
    </div>
  )
}

function EmptyRecords({ kind }: { kind: Kind }) {
  if (kind === 'scene')
    return (
      <div class="empty">
        No cluster scene yet. <b>New scene</b> creates one on this node; <b>Same numbers</b> links this node's scenes
        to cluster scenes of the same number.
      </div>
    )
  return (
    <div class="empty">
      No channel in the cluster yet. <b>New channel</b> creates one on this node; <b>From this node's groups…</b> makes
      channels of this node's groups.
    </div>
  )
}

function Outcomes({ outcomes, onClose }: { outcomes: Outcome[] | null; onClose: () => void }) {
  if (!outcomes || outcomes.length === 0) return null
  return (
    <div class="outcomes" role="status">
      {outcomes.map((o, i) => (
        <div class="planrow" key={i}>
          <span class="mono">{o.item}</span>
          <span class="what">{o.text}</span>
          <Chip cls={o.ok ? 'ok' : 'err'}>{o.ok ? 'done' : 'refused'}</Chip>
        </div>
      ))}
      <div class="planrow">
        <span class="mono" />
        <span class="what" />
        <button class="btn sm ghost" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  )
}

function ApplyBar({ draft, busy, onDiscard, onApply }: {
  draft: Draft
  busy: boolean
  onDiscard: () => void
  onApply: () => void
}) {
  const items = draftItems(draft)
  if (items.length === 0) return null
  return (
    <div class="footer-bar">
      <span class="n">{draftLine(items)}</span>
      <span class="spacer" />
      <button class="btn ghost" disabled={busy} onClick={onDiscard}>
        Discard
      </button>
      <button class="btn primary" disabled={busy} onClick={onApply}>
        Apply
      </button>
    </div>
  )
}

function Head({ world, status }: { world: World | null; status: ClusterStatus | null }) {
  const line = nodeLine(status)
  const on = world?.settings.enabled
  const sub = !world ? null : on && line ? `${world.settings.cluster_name} · ${line.text}` : on ? world.settings.cluster_name : 'cluster off'
  return (
    <div class="head">
      <h1>Cluster</h1>
      {sub && <span class="sub">{sub}</span>}
    </div>
  )
}

function Tabs({ kind, world }: { kind: Kind; world: World }) {
  return (
    <>
      <div class="ptabs">
        {(['channel', 'scene'] as const).map((k) => (
          <a key={k} class={k === kind ? 'ptab active' : 'ptab'} href={TAB_HREF[k]}>
            {TAB_TITLE[k]}
            <span class="hint">{world.records[k].length}</span>
          </a>
        ))}
      </div>
      <div class="ptabline" />
    </>
  )
}

function OffEmpty() {
  return (
    <div class="card">
      <div class="empty">
        This controller is not part of a cluster. Channels and scenes of the cluster appear here once the cluster is on.
        <br />
        <br />
        <a class="btn sm" href={SETTINGS_HREF}>
          Cluster settings
        </a>
      </div>
    </div>
  )
}

function Banners({ world, peerUrl }: { world: World; peerUrl: string | null }) {
  return (
    <>
      {world.settings.read_only && (
        <StandbyBanner peerUrl={peerUrl}>
          This board is the standby of its pair: channels and scenes are shown here, not switched or edited.
        </StandbyBanner>
      )}
      {!world.settings.enabled && (
        <p class="req">
          The cluster is off. Channels and scenes recorded on this node can still be edited; On, Off and Recall are
          refused until it is turned on in <a href={SETTINGS_HREF}>Cluster settings</a>.
        </p>
      )}
    </>
  )
}

function hasAnything(world: World): boolean {
  return (['channel', 'scene'] as const).some((k) => world.records[k].length + world.own[k].length > 0)
}

function useOpened(kind: Kind, open: string | undefined) {
  const [opened, setOpened] = useState<Set<string>>(() => new Set(open ? [`${kind}:${open}`] : []))
  useEffect(() => {
    if (open) setOpened((now) => new Set(now).add(`${kind}:${open}`))
  }, [kind, open])
  const flip = (n: number) =>
    setOpened((now) => {
      const next = new Set(now)
      const key = `${kind}:${n}`
      if (!next.delete(key)) next.add(key)
      return next
    })
  const add = (n: number) => setOpened((now) => new Set(now).add(`${kind}:${n}`))
  return { isOpen: (n: number) => opened.has(`${kind}:${n}`), toggle: flip, openRow: add }
}

function useCommands(reload: () => void) {
  const [rowErr, setRowErr] = useState<Record<string, RowErr>>({})
  const command = (kind: Kind, rec: ClusterRecord, title: string, send: () => Promise<unknown>) => {
    const key = `${kind}:${rec.number}`
    setRowErr((now) => without(now, key))
    send()
      .then(() => notify(title, 'succeeded', 'Sent'))
      .catch((e) => {
        setRowErr((now) => ({ ...now, [key]: rowErrOf(e) }))
        if (e instanceof ApiError && e.status === 404) reload()
      })
  }
  return { rowErr, command }
}

const APPLY_PORTS: ApplyPorts = {
  rename: (kind, number, name, revision) => api.renameClusterRecord(kind, number, name, revision),
  unlink: (kind, local) => api.deleteClusterPart(kind, ADAPTER, local),
  link: (kind, local, body) => api.putClusterPart(kind, ADAPTER, local, body),
  refusal: (e) => refusalFrom(e, 'records').text,
}

function useRecordDraft(world: World | null, reload: () => void) {
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT)
  const [errs, setErrs] = useState<Record<string, FieldErrors>>({})
  const [outcomes, setOutcomes] = useState<Outcome[] | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    if (world) setDraft((now) => draftOnRead(now, world.records, world.own, world.settings.read_only))
  }, [world])
  const edit = (fn: (d: Draft) => Draft, key: string) => {
    setDraft(fn)
    setErrs((now) => without(now, key))
  }
  const setErr = (key: string, what: keyof FieldErrors, text: string) =>
    setErrs((now) => ({ ...now, [key]: { ...now[key], [what]: text } }))
  const apply = async () => {
    if (!world) return
    setBusy(true)
    const out = await runApply(planApply(draft, world.records, world.own), APPLY_PORTS)
    setOutcomes(out)
    setErrs(fieldErrors(out))
    setDraft((now) => draftAfter(now, out))
    setBusy(false)
    reload()
  }
  const discard = () => {
    setDraft(EMPTY_DRAFT)
    setErrs({})
    setOutcomes(null)
  }
  return { draft, edit, errs, setErr, outcomes, busy, apply, discard, closeOutcomes: () => setOutcomes(null) }
}

function Page({ children }: { children: ComponentChildren }) {
  return <div>{children}</div>
}

const SCENES_HINT =
  "A cluster scene recalls, on each node, the scene chosen for it there. What a scene sets is edited in that node's scene editor."

function Loaded({ ctx, rd, peerUrl }: { ctx: Ctx; rd: ReturnType<typeof useRecordDraft>; peerUrl: string | null }) {
  return (
    <Page>
      <Head world={ctx.world} status={ctx.status} />
      <Banners world={ctx.world} peerUrl={peerUrl} />
      <Tabs kind={ctx.kind} world={ctx.world} />
      {ctx.kind === 'scene' && <p class="hint">{SCENES_HINT}</p>}
      <Outcomes outcomes={rd.outcomes} onClose={rd.closeOutcomes} />
      <RecordsCard ctx={ctx} />
      {!ctx.readOnly && (
        <ApplyBar draft={rd.draft} busy={rd.busy} onDiscard={rd.discard} onApply={() => void rd.apply()} />
      )}
    </Page>
  )
}

function Waiting({ world, status, error }: { world: World | null; status: ClusterStatus | null; error: string | null }) {
  return (
    <Page>
      <Head world={world} status={status} />
      {world ? (
        <OffEmpty />
      ) : (
        <div class="empty">{error ? `Cluster records cannot be read: ${error}` : 'Loading the cluster…'}</div>
      )}
    </Page>
  )
}

export function ClusterControl({ tab, open }: { tab?: string; open?: string }) {
  const kind: Kind = tab === 'scenes' ? 'scene' : 'channel'
  const { data: world, error, reload } = useLive(readWorld, ['cluster'], { onEvent: statusOnly })
  const status = useClusterStatus().read?.status ?? null
  const peerUrl = usePeerUrl(world?.settings.read_only ?? false)
  const rows = useOpened(kind, open)
  const { rowErr, command } = useCommands(() => void reload())
  const rd = useRecordDraft(world, () => void reload())
  const [armed, tap] = useConfirmTap<string>()
  if (!world || (!world.settings.enabled && !hasAnything(world)))
    return <Waiting world={world} status={status} error={error} />
  const ctx: Ctx = {
    world, kind, me: world.settings.node_number, readOnly: world.settings.read_only, busy: rd.busy, status,
    draft: rd.draft, edit: rd.edit, errs: rd.errs, setErr: rd.setErr, rowErr,
    command: (rec, title, send) => command(kind, rec, title, send),
    armed, tap, reload: () => void reload(), ...rows,
  }
  return <Loaded ctx={ctx} rd={rd} peerUrl={peerUrl} />
}
