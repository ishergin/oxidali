import { useEffect, useRef, useState } from 'preact/hooks'
import { ApiError, api } from '../api/client'
import type { ClusterInputs, ClusterInputsNode, ClusterStatus } from '../api/types'
import type { WsEvent } from '../api/ws'
import { useLive } from '../hooks'
import {
  type InputRow,
  type LastShown,
  type LiveEvent,
  type LivePayload,
  type NodeGroup,
  type NodeRead,
  PENDING,
  filterGroups,
  inputNodes,
  liveOf,
  movedNodes,
  newerRead,
  nodeGroup,
  totalsText,
} from './cluster-inputs-view'

const TICK_MS = 1000
const INPUTS_FRAME = 'ClusterInputs'
const LIVE_TYPES: ReadonlySet<string> = new Set(['ClusterInputEventObservedEvent', 'DaliInputEventObservedEvent'])
const OWN_DEVICE_TYPES: ReadonlySet<string> = new Set(['InputDeviceChangedEvent', 'DaliInputDeviceLifecycleEvent'])
const HTTP_NOT_FOUND = 404
const COLUMNS = 5

async function readNode(node: number): Promise<NodeRead> {
  const at = Date.now()
  try {
    return { kind: 'read', data: await api.clusterNodeInputs(node), at }
  } catch (e) {
    return e instanceof ApiError && e.status === HTTP_NOT_FOUND ? { kind: 'absent', at } : { kind: 'failed', at }
  }
}

function useNow(): number {
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), TICK_MS)
    return () => clearInterval(id)
  }, [])
  return now
}

function useNewNodes(nodes: number[], onNew: (fresh: number[]) => void) {
  const known = useRef<Set<number> | null>(null)
  const key = nodes.join(',')
  useEffect(() => {
    const before = known.current
    known.current = new Set(nodes)
    const fresh = before ? nodes.filter((n) => !before.has(n)) : []
    if (fresh.length > 0) onNew(fresh)
  }, [key])
}

function useInputsFeed(me: number, status: ClusterStatus | null) {
  const [reads, setReads] = useState<Record<number, NodeRead>>({})
  const [live, setLive] = useState<Map<string, LiveEvent>>(new Map())
  const [frame, setFrame] = useState<ClusterInputsNode[] | null>(null)
  const frameRef = useRef<ClusterInputsNode[] | null>(null)
  const nodes = inputNodes(me, status, frame)
  const nodesRef = useRef(nodes)
  nodesRef.current = nodes
  const fetchNodes = async (list: number[]) => {
    const got = await Promise.all(list.map(readNode))
    setReads((now) => {
      const next = { ...now }
      list.forEach((n, i) => (next[n] = newerRead(now[n], got[i])))
      return next
    })
  }
  const onEvent = (event: WsEvent): boolean => {
    if (event.type === INPUTS_FRAME && event.payload) {
      const after = (event.payload as ClusterInputs).nodes
      const moved = movedNodes(frameRef.current, after)
      frameRef.current = after
      setFrame(after)
      if (moved.length > 0) void fetchNodes(moved)
    } else if (LIVE_TYPES.has(event.type) && event.payload) {
      const entry = liveOf(event.payload as LivePayload, me, Date.now())
      if (entry) setLive((now) => new Map(now).set(entry[0], entry[1]))
    } else if (OWN_DEVICE_TYPES.has(event.type)) {
      void fetchNodes([me])
    }
    return true
  }
  useLive(() => fetchNodes(nodesRef.current), ['cluster', 'input'], { onEvent, deps: [me] })
  useNewNodes(nodes, (fresh) => void fetchNodes(fresh))
  return { nodes, reads, live }
}

function Last({ last }: { last: LastShown }) {
  if (last.kind === 'none') return <span class="unit">nothing since boot</span>
  if (last.kind === 'named') return <>{last.text}</>
  return (
    <>
      <span class="mono">{last.code}</span>
      <span class="unit"> not named</span>
    </>
  )
}

function Row({ row, stale }: { row: InputRow; stale: boolean }) {
  return (
    <tr class={stale ? 'lastheard' : undefined}>
      <td class="num">{row.address}</td>
      <td class="nm-col">
        {row.name ? <span class="nm">{row.name}</span> : <span class="nm name-faint">— unnamed</span>}
        {row.haHidden && <span class="unit"> hidden from Home Assistant</span>}
      </td>
      <td class="unit">{row.type}</td>
      <td>
        <Last last={row.last} />
      </td>
      <td class="unit">{row.when}</td>
    </tr>
  )
}

function Group({ group }: { group: NodeGroup }) {
  return (
    <>
      <tr class="group-row">
        <td colSpan={COLUMNS}>
          {group.head}
          <span class="unit">{group.note}</span>
        </td>
      </tr>
      {group.rows.map((r) => (
        <Row key={r.key} row={r} stale={group.stale} />
      ))}
    </>
  )
}

function InputsTable({ groups, query }: { groups: NodeGroup[]; query: string }) {
  return (
    <div class="card">
      <div class="table-wrap">
        <table class="ctable">
          <thead>
            <tr>
              <th>Input</th>
              <th>Name</th>
              <th>Type</th>
              <th>Last event</th>
              <th>When</th>
            </tr>
          </thead>
          <tbody>
            {groups.map((g) => (
              <Group key={g.node} group={g} />
            ))}
          </tbody>
        </table>
      </div>
      {query.trim() !== '' && groups.every((g) => g.rows.length === 0) && (
        <div class="empty">No input device of the nodes read matches “{query.trim()}”.</div>
      )}
    </div>
  )
}

export function ClusterInputsTab({ me, share, status }: { me: number; share: boolean; status: ClusterStatus | null }) {
  const { nodes, reads, live } = useInputsFeed(me, status)
  const [query, setQuery] = useState('')
  const now = useNow()
  const ctx = { me, share, status, live, now }
  const shown = filterGroups(
    nodes.map((n) => nodeGroup(n, reads[n] ?? PENDING, ctx)),
    query,
  )
  return (
    <>
      <div class="toolbar">
        <input
          class="search"
          placeholder="Search all nodes by name or address"
          aria-label="Search the input devices of all nodes"
          value={query}
          onInput={(e) => setQuery(e.currentTarget.value)}
        />
        <span class="unit">{totalsText(shown)}</span>
      </div>
      <InputsTable groups={shown} query={query} />
    </>
  )
}
