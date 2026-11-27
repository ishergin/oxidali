import type { ComponentChildren } from 'preact'
import { useEffect, useState } from 'preact/hooks'
import { ApiError, api } from '../api/client'
import type { ClusterInfo, ClusterNode, ClusterSettings, ClusterStatus } from '../api/types'
import { type StatusRead, useClusterInfo, useClusterStatus } from '../cluster-feed'
import { Card, Chip, EditableText, FieldRow, SelChip, Switch, useConfirmTap } from '../components/ui'
import { useSettingsDraft, useTick } from '../hooks'
import { errorMessage, notify } from '../toast'
import {
  type ClusterField,
  type Draft,
  NO_DRAFT,
  type Option,
  type Refusal,
  type Tone,
  type Verdict,
  clipName,
  draftOnRead,
  editDraft,
  executorVerdict,
  forgetLabel,
  forgetRefusal,
  forgettable,
  heardVerdict,
  ignoredReason,
  nodeHref,
  nodeNumberOptions,
  nodesEmptyText,
  nodeSummary,
  ownVerdict,
  parsePort,
  pinReason,
  refusalOf,
  settingsShown,
  showsHomeAssistant,
  unsavedLine,
  wireOptions,
} from './cluster-view'
import { fieldPatch, sendFieldsOnFreshRead, withoutSent } from './field-edits'

type Edit = (change: Partial<ClusterSettings>) => void

const CHIP: Record<Tone, string> = { plain: 'muted', ok: 'ok', warn: 'warn', err: 'err' }

function VerdictChip({ v }: { v: Verdict }) {
  return <Chip cls={CHIP[v.tone]}>{v.text}</Chip>
}

function refusalFrom(e: unknown): Refusal {
  if (e instanceof ApiError) return refusalOf(e.status, e.code, e.message)
  return { field: null, text: errorMessage(e) }
}

function FieldError({ refusal, field }: { refusal: Refusal | null; field: ClusterField }) {
  if (refusal?.field !== field) return null
  return (
    <span class="fielderr" role="alert">
      {refusal.text}
    </span>
  )
}

function Select({
  value,
  options,
  disabled,
  bad,
  label,
  onPick,
}: {
  value: number | null
  options: Option[]
  disabled: boolean
  bad: boolean
  label: string
  onPick: (value: number | null) => void
}) {
  return (
    <select
      class={bad ? 'sel err' : 'sel'}
      aria-label={label}
      disabled={disabled}
      value={value === null ? '' : String(value)}
      onChange={(e) => {
        const raw = e.currentTarget.value
        onPick(raw === '' ? null : Number(raw))
      }}
    >
      {options.map((o) => (
        <option key={o.label} value={o.value === null ? '' : String(o.value)}>
          {o.label}
        </option>
      ))}
    </select>
  )
}

function Dim({ on, children }: { on: boolean; children: ComponentChildren }) {
  return on ? (
    <div class="ro" inert>
      {children}
    </div>
  ) : (
    <>{children}</>
  )
}

interface PanelProps {
  s: ClusterSettings
  edit: Edit
  refusal: Refusal | null
  readOnly: boolean
}

function ThisNode({
  s,
  edit,
  refusal,
  readOnly,
  status,
  info,
  storedPort,
}: PanelProps & { status: ClusterStatus | null; info: ClusterInfo | null; storedPort: number }) {
  const own = status?.own
  return (
    <Card title="This node">
      <FieldRow
        label="Enabled"
        hint="Off by default. Talks to the other nodes over this network; only the active board of a pair transmits."
      >
        <Switch on={s.enabled} onToggle={() => edit({ enabled: !s.enabled })} />
      </FieldRow>
      <FieldRow
        label="Cluster name"
        hint="Required to enable. Boards with another name are ignored, and listed below."
      >
        <EditableText
          cls={refusal?.field === 'cluster_name' ? 'txt err' : 'txt'}
          value={s.cluster_name}
          label="Cluster name"
          disabled={readOnly}
          onCommit={(raw) => edit({ cluster_name: clipName(raw) })}
        />
        <FieldError refusal={refusal} field="cluster_name" />
      </FieldRow>
      <NodeNumberRow s={s} edit={edit} refusal={refusal} readOnly={readOnly} status={status} info={info} />
      <PortRow s={s} edit={edit} refusal={refusal} readOnly={readOnly} />
      <FieldRow label="Status" hint={own?.board ? <span class="mono">{own.board}</span> : undefined}>
        {own ? <VerdictChip v={ownVerdict(own, storedPort)} /> : <span class="unit">reading…</span>}
      </FieldRow>
    </Card>
  )
}

function NodeNumberRow({
  s,
  edit,
  refusal,
  readOnly,
  status,
  info,
}: PanelProps & { status: ClusterStatus | null; info: ClusterInfo | null }) {
  const pinned = pinReason(info)
  return (
    <FieldRow label="Node number" hint={pinned ?? '0–7, one per installation. Both boards of a pair share it.'}>
      <Select
        label="Node number"
        value={s.node_number}
        options={nodeNumberOptions(status, s.node_number)}
        disabled={readOnly || pinned !== null}
        bad={refusal?.field === 'node_number'}
        onPick={(n) => n !== null && edit({ node_number: n })}
      />
      <FieldError refusal={refusal} field="node_number" />
    </FieldRow>
  )
}

function PortRow({ s, edit, readOnly }: PanelProps) {
  return (
    <FieldRow label="Port" hint="UDP, 1–65535, the same on every node.">
      <EditableText
        cls="num"
        inputMode="numeric"
        value={String(s.port)}
        label="Port"
        disabled={readOnly}
        onCommit={(raw) => {
          const port = parsePort(raw)
          if (port !== null) edit({ port })
        }}
      />
    </FieldRow>
  )
}

function NodeRow({
  node,
  status,
  readOnly,
  armed,
  sinceReadMs,
  onForget,
}: {
  node: ClusterNode
  status: ClusterStatus
  readOnly: boolean
  sinceReadMs: number
  armed: boolean
  onForget: () => void
}) {
  return (
    <tr>
      <td class="mono">{node.node}</td>
      <td class="mono">{node.board}</td>
      <td class="mono">{node.address}</td>
      <td>
        <VerdictChip v={heardVerdict(node, sinceReadMs)} />
      </td>
      <td>
        {nodeSummary(node, status.own).map((v) => (
          <VerdictChip key={v.text} v={v} />
        ))}
        {showsHomeAssistant(node) && <span class="unit"> shows the cluster in Home Assistant</span>}
      </td>
      <td class="acts-col">
        <RowActions node={node} readOnly={readOnly} armed={armed} onForget={onForget} />
      </td>
    </tr>
  )
}

function RowActions({
  node,
  readOnly,
  armed,
  onForget,
}: {
  node: ClusterNode
  readOnly: boolean
  armed: boolean
  onForget: () => void
}) {
  return (
    <span class="acts">
      <a
        class="btn sm"
        href={nodeHref(node)}
        target="_blank"
        rel="noopener noreferrer"
        aria-label={`Open node ${node.node}`}
      >
        Open
      </a>
      {forgettable(node, readOnly) && (
        <button
          class={armed ? 'btn sm sure' : 'btn sm danger'}
          aria-label={`${forgetLabel(armed)} node ${node.node}`}
          onClick={onForget}
        >
          {forgetLabel(armed)}
        </button>
      )}
    </span>
  )
}

const AGE_TICK_MS = 1000

function NodesHead() {
  return (
    <thead>
      <tr>
        <th>Node</th>
        <th>Board</th>
        <th>Address</th>
        <th>Heard</th>
        <th>Settings</th>
        <th class="acts-col" />
      </tr>
    </thead>
  )
}

async function forgetNode(node: number): Promise<void> {
  const title = `Forget node ${node}`
  try {
    await api.forgetClusterNode(node)
    notify(title, 'succeeded', 'Forgotten')
  } catch (e) {
    const status = e instanceof ApiError ? e.status : undefined
    notify(title, 'failed', forgetRefusal(node, status, e instanceof ApiError ? e.message : errorMessage(e)))
  }
}

function NodesHeard({
  read,
  readOnly,
  reload,
}: {
  read: StatusRead | null
  readOnly: boolean
  reload: () => Promise<void>
}) {
  const [armed, tap] = useConfirmTap<string>()
  const now = useTick(AGE_TICK_MS)
  const status = read?.status ?? null
  const forget = (node: number) => void forgetNode(node).then(reload)
  return (
    <Card title="Nodes heard">
      {!status || status.nodes.length === 0 ? (
        <p class="empty">{nodesEmptyText(status)}</p>
      ) : (
        <div class="table-wrap">
          <table>
            <NodesHead />
            <tbody>
              {status.nodes.map((node) => (
                <NodeRow
                  key={node.node}
                  node={node}
                  status={status}
                  readOnly={readOnly}
                  armed={armed === String(node.node)}
                  sinceReadMs={read ? now - read.at : 0}
                  onForget={() => tap(String(node.node), () => forget(node.node))}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </Card>
  )
}

function Ignored({ status }: { status: ClusterStatus | null }) {
  if (!status || status.ignored.length === 0) return null
  return (
    <Card title="Heard, but not part of this cluster">
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Board</th>
              <th>Address</th>
              <th>Why it is ignored</th>
            </tr>
          </thead>
          <tbody>
            {status.ignored.map((row) => {
              const why = ignoredReason(row, status)
              return (
                <tr key={row.board}>
                  <td class="mono">{row.board}</td>
                  <td class="mono">{row.address}</td>
                  <td>
                    {why.text}
                    {why.name !== null && <span class="mono">{why.name}</span>}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </Card>
  )
}

const SOURCES: [keyof ClusterSettings & `forward_${string}`, string][] = [
  ['forward_operator', 'Operator'],
  ['forward_home_assistant', 'Home Assistant'],
  ['forward_hcl', 'HCL'],
  ['forward_foreign_master', 'Foreign master'],
]

function Forwarding({ s, edit }: PanelProps) {
  return (
    <Card title="Forward group commands given by">
      <FieldRow
        stacked
        label="Sources"
        hint="Commands to this node's own groups go to the cluster when their source is on."
      >
        <div class="chips">
          {SOURCES.map(([key, label]) => (
            <SelChip key={key} on={s[key]} onClick={() => edit({ [key]: !s[key] })}>
              {label}
            </SelChip>
          ))}
        </div>
      </FieldRow>
    </Card>
  )
}

function ShareAccept({ s, edit }: PanelProps) {
  return (
    <Card title="Share and accept">
      <FieldRow
        label="Send “all lamps” commands"
        hint="A broadcast given on this node's DALI bus is sent to every node."
      >
        <Switch on={s.send_all_lamps} onToggle={() => edit({ send_all_lamps: !s.send_all_lamps })} />
      </FieldRow>
      <FieldRow
        label="Accept “all lamps” commands"
        hint="A cluster-wide “all” becomes a broadcast on this node's DALI bus."
      >
        <Switch
          on={s.accept_all_lamps}
          onToggle={() => edit({ accept_all_lamps: !s.accept_all_lamps })}
        />
      </FieldRow>
    </Card>
  )
}

function DaliBus({ s, edit, refusal, readOnly, status }: PanelProps & { status: ClusterStatus | null }) {
  return (
    <Card title="DALI bus">
      <FieldRow
        label="This node's DALI bus"
        hint="Leave it on “own bus” unless another node is wired to the same DALI bus. One node of a shared bus keeps “own bus”; the others choose that node."
      >
        <Select
          label="DALI bus"
          value={s.wire_same_as}
          options={wireOptions(status, s.node_number, s.wire_same_as)}
          disabled={readOnly}
          bad={refusal?.field === 'wire_same_as'}
          onPick={(wire) => edit({ wire_same_as: wire })}
        />
        <FieldError refusal={refusal} field="wire_same_as" />
      </FieldRow>
      <FieldRow
        label="On the shared bus"
        hint="Only one node of a bus carries out cluster commands and forwards the foreign master."
      >
        {status ? <VerdictChip v={executorVerdict(status.own)} /> : <span class="unit">reading…</span>}
      </FieldRow>
      <FieldRow
        label="Loop protection"
        hint="Cannot be turned off while the foreign master is forwarded and the bus is shared."
      >
        <Switch on={s.wire_protection} onToggle={() => edit({ wire_protection: !s.wire_protection })} />
        <FieldError refusal={refusal} field="wire_protection" />
      </FieldRow>
    </Card>
  )
}

function ApplyBar({
  patch,
  busy,
  onDiscard,
  onApply,
}: {
  patch: Partial<ClusterSettings>
  busy: boolean
  onDiscard: () => void
  onApply: () => void
}) {
  const dirty = Object.keys(patch).length > 0
  return (
    <div class={dirty ? 'footer-bar' : 'footer-bar idle'}>
      <span class="n">{unsavedLine(patch)}</span>
      <span class="spacer" />
      <button class="btn ghost" disabled={busy || !dirty} onClick={onDiscard}>
        Discard
      </button>
      <button class="btn primary" disabled={busy || !dirty} onClick={onApply}>
        Apply
      </button>
    </div>
  )
}

function StandbyBanner({ peerUrl }: { peerUrl: string | null }) {
  return (
    <p class="req">
      This board is the standby of its pair: the settings come from the active board and are
      replaced every 30 s.{' '}
      {peerUrl ? (
        <a href={peerUrl} target="_blank" rel="noopener noreferrer">
          Open the active board
        </a>
      ) : (
        'Open the active board'
      )}{' '}
      to change them.
    </p>
  )
}

function usePeerUrl(readOnly: boolean): string | null {
  const [peer, setPeer] = useState<string | null>(null)
  useEffect(() => {
    if (!readOnly) return
    api
      .redundancySettings()
      .then((r) => setPeer(r.peer_url || null))
      .catch(() => setPeer(null))
  }, [readOnly])
  return peer
}

interface ClusterDraft {
  draft: Draft
  edit: Edit
  discard: () => void
  apply: () => void
}

function useClusterDraft(
  data: ClusterSettings | null,
  busy: boolean,
  save: (title: string, send: () => Promise<unknown>) => Promise<void>,
): ClusterDraft {
  const [draft, setDraft] = useState<Draft>(NO_DRAFT)
  useEffect(() => {
    if (data) setDraft((now) => draftOnRead(now, data))
  }, [data])
  const edit: Edit = (change) => {
    if (data) setDraft((now) => editDraft(now, change, data, busy))
  }
  const apply = () =>
    void save('Cluster settings', async () => {
      setDraft((now) => ({ ...now, refusal: null }))
      try {
        const sent = await sendFieldsOnFreshRead(draft.edits ?? {}, api.clusterSettings, api.patchClusterSettings)
        setDraft((now) => ({ ...now, edits: now.edits && withoutSent(now.edits, sent) }))
      } catch (e) {
        setDraft((now) => ({ ...now, refusal: refusalFrom(e) }))
        throw e
      }
    })
  return { draft, edit, discard: () => setDraft(NO_DRAFT), apply }
}

function ClusterHead() {
  return (
    <div class="head">
      <h1>Cluster</h1>
      <span class="sub">
        Up to eight controllers in one Ethernet segment act as one lighting system. Channels and
        scenes of the cluster live under Control; this page holds this node's own parameters.
      </span>
    </div>
  )
}

export function SettingsCluster() {
  const { data, busy, save } = useSettingsDraft(() => api.clusterSettings())
  const { read, reload } = useClusterStatus()
  const { info } = useClusterInfo()
  const { draft, edit, discard, apply } = useClusterDraft(data, busy, save)
  const peerUrl = usePeerUrl(data?.read_only ?? false)
  if (!data) return <div class="empty">Loading cluster settings…</div>
  const readOnly = data.read_only
  const status = read?.status ?? null
  const refusal = draft.refusal
  const patch = draft.edits && !readOnly ? fieldPatch(draft.edits, data) : {}
  const props: PanelProps = { s: settingsShown(draft, data), edit, refusal, readOnly }

  return (
    <div>
      <ClusterHead />
      {readOnly && <StandbyBanner peerUrl={peerUrl} />}
      {refusal && (
        <p class="errbar" role="alert">
          Not applied: {refusal.text}
        </p>
      )}
      <Dim on={readOnly}>
        <ThisNode {...props} status={status} info={info} storedPort={data.port} />
      </Dim>
      <NodesHeard read={read} readOnly={readOnly} reload={reload} />
      <Ignored status={status} />
      <Dim on={readOnly}>
        <Forwarding {...props} />
        <ShareAccept {...props} />
        <DaliBus {...props} status={status} />
      </Dim>
      {!readOnly && (
        <ApplyBar patch={patch} busy={busy} onDiscard={discard} onApply={apply} />
      )}
    </div>
  )
}
