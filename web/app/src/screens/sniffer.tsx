import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'preact/hooks'

import { connection, subscribe, type WsEvent } from '../api/ws'
import { deviceClock, UNANCHORED_CLOCK_HINT } from '../format'
import { anchorIndex, isAtBottom, windowBounds } from './follow-scroll'
import { Card } from '../components/ui'

const RING_CAPACITY = 2000

const RENDER_WINDOW = 400

const RATE_WINDOW_MS = 1000

const RATE_TICK_MS = 500

interface SnifferFrame {
  ts_ms: number
  dir: 'tx' | 'rx' | 'reply'
  width: string
  adapter_id: number
  attempt: number
  hex: string
  target: { kind: string; id?: number } | null
  name: string
  detail: string | null
  is_query: boolean
}

interface SnifferBatch {
  dropped_since: number
  frames: SnifferFrame[]
}

interface FrameRowData {
  kind: 'frame'
  seq: number
  frame: SnifferFrame
  time: string
  anchored: boolean
  target: string
  hay: string
}

type Row = FrameRowData | { kind: 'gap'; seq: number; lost: number }

interface RenderedRow {
  row: Row
  delta: number | null
}

type DirFilter = 'all' | 'tx' | 'rx' | 'reply'

const DIR_GLYPH: Record<string, string> = { tx: '→', rx: '↔', reply: '←' }

function targetLabel(target: SnifferFrame['target']): string {
  if (!target) return '—'
  switch (target.kind) {
    case 'short':
      return `short ${target.id}`
    case 'group':
      return `group ${target.id}`
    case 'broadcast':
      return 'broadcast'
    case 'broadcast_unaddressed':
      return 'broadcast (unaddressed)'
    default:
      return target.id === undefined ? target.kind : `${target.kind} ${target.id}`
  }
}

function asText(rows: Row[]): string {
  return rows
    .map((row) =>
      row.kind === 'gap'
        ? `--- ${row.lost} frame(s) dropped ---`
        : [
            row.time,
            DIR_GLYPH[row.frame.dir] ?? '?',
            row.frame.hex.padEnd(8),
            row.frame.name,
            row.frame.detail ?? '',
            row.target,
          ].join('  '),
    )
    .join('\n')
}

export function Sniffer() {
  const [running, setRunning] = useState(false)
  const [paused, setPaused] = useState(false)
  const [follow, setFollow] = useState(true)
  const logRef = useRef<HTMLDivElement | null>(null)
  const anchor = useRef<number | null>(null)
  const [dir, setDir] = useState<DirFilter>('all')
  const [hideQueries, setHideQueries] = useState(false)
  const [needle, setNeedle] = useState('')
  const [rows, setRows] = useState<Row[]>([])
  const [dropped, setDropped] = useState(0)
  const [rate, setRate] = useState(0)

  const seq = useRef(0)
  const pausedRef = useRef(paused)
  pausedRef.current = paused
  const recent = useRef<number[]>([])

  const resetRate = () => {
    recent.current = []
    setRate(0)
  }

  useEffect(() => {
    if (!running) return
    const id = setInterval(() => {
      const now = Date.now()
      recent.current = recent.current.filter((t) => now - t < RATE_WINDOW_MS)
      setRate(recent.current.length)
    }, RATE_TICK_MS)
    return () => clearInterval(id)
  }, [running])

  useEffect(() => {
    if (!running) return
    const dispose = subscribe(['sniffer'], (event: WsEvent) => {
      if (event.type === 'DropNotice') {
        const lost = (event as unknown as { dropped_count?: number }).dropped_count ?? 0
        setDropped((d) => d + lost)
        return
      }
      if (event.type !== 'SnifferBatch') return
      const batch = event.payload as SnifferBatch
      if (batch.dropped_since > 0) setDropped((d) => d + batch.dropped_since)
      const now = Date.now()
      recent.current = [
        ...recent.current.filter((t) => now - t < RATE_WINDOW_MS),
        ...batch.frames.map(() => now),
      ]
      setRate(recent.current.length)
      if (pausedRef.current) return
      setRows((prev) => appendBatch(prev, batch, seq))
    })
    return dispose
  }, [running])

  const view = useMemo(() => {
    const needleLc = needle.trim().toLowerCase()
    const shown = rows.filter((row) => matches(row, dir, hideQueries, needleLc))
    const held = anchorIndex(shown.map((row) => row.seq), anchor.current)
    const { start, end, newer } = windowBounds(shown.length, held, RENDER_WINDOW)
    let previous: SnifferFrame | null = null
    for (let i = start - 1; i >= 0; i -= 1) {
      const row = shown[i]
      if (row.kind === 'frame') {
        previous = row.frame
        break
      }
    }
    const rendered: RenderedRow[] = []
    for (let i = start; i < end; i += 1) {
      const row = shown[i]
      if (row.kind === 'gap') {
        rendered.push({ row, delta: null })
        continue
      }
      rendered.push({ row, delta: previous ? row.frame.ts_ms - previous.ts_ms : null })
      previous = row.frame
    }
    return { shown, rendered, hidden: start, newer }
  }, [rows, dir, hideQueries, needle, follow])

  useLayoutEffect(() => {
    if (!follow) return
    const el = logRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [view.rendered, follow])

  const onLogScroll = (event: Event) => {
    const el = event.currentTarget as HTMLDivElement
    const atBottom = isAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight)
    if (atBottom) {
      anchor.current = null
    } else if (anchor.current === null) {
      anchor.current = newestSeq()
    }
    setFollow(atBottom)
  }

  const newestSeq = () => view.shown[view.shown.length - 1]?.seq ?? null

  const setFollowing = (on: boolean) => {
    anchor.current = on ? null : newestSeq()
    setFollow(on)
  }

  return (
    <>
      <div class="crumbs">
        <a href="#/">Adapter 0</a> / DALI sniffer
      </div>
      <div class="head">
        <h1>DALI sniffer</h1>
        <span class="spacer" />
        <span class="sub">{running ? 'capturing' : 'stopped'}</span>
      </div>
      <p class="hint">
        Every frame on the wire — ours, the other master's, and the answers — decoded as it
        happens. Capturing costs the controller nothing while this is stopped.
      </p>
      <Card title="Bus traffic">
        <div class="snf-bar">
          <button
            class={running ? 'btn stop' : 'btn primary'}
            onClick={() => {
              if (running) resetRate()
              setRunning(!running)
            }}
          >
            {running ? 'Stop' : 'Start'}
          </button>
          <button class="btn ghost" disabled={!running} onClick={() => setPaused((p) => !p)}>
            {paused ? 'Resume' : 'Pause'}
          </button>
          <button
            class="btn ghost"
            onClick={() => {
              setRows([])
              setDropped(0)
              resetRate()
              setFollowing(true)
            }}
          >
            Clear
          </button>
          <div class="snf-sep" />
          <div class="snf-filter">
            {(['all', 'tx', 'rx', 'reply'] as DirFilter[]).map((d) => (
              <button class={dir === d ? 'on' : undefined} onClick={() => setDir(d)}>
                {d === 'all' ? 'All' : d.toUpperCase()}
              </button>
            ))}
          </div>
          <div class="snf-filter">
            <button class={hideQueries ? undefined : 'on'} onClick={() => setHideQueries(false)}>
              All frames
            </button>
            <button class={hideQueries ? 'on' : undefined} onClick={() => setHideQueries(true)}>
              Hide queries
            </button>
          </div>
          <div class="snf-filter">
            <button class={follow ? 'on' : undefined} onClick={() => setFollowing(true)}>
              Follow
            </button>
            <button class={follow ? undefined : 'on'} onClick={() => setFollowing(false)}>
              Manual
            </button>
          </div>
          <input
            class="snf-search"
            placeholder="addr / hex"
            value={needle}
            onInput={(e) => setNeedle((e.target as HTMLInputElement).value)}
          />
          <div class="snf-sep" />
          <button
            class="btn ghost"
            disabled={!view.shown.length}
            onClick={() => void navigator.clipboard?.writeText(asText(view.shown))}
          >
            Copy
          </button>
        </div>
        {running ? (
          <FrameLog
            rendered={view.rendered}
            hidden={view.hidden}
            logRef={logRef}
            onScroll={onLogScroll}
          />
        ) : (
          <IdleNote />
        )}
        <div class="snf-foot">
          <span>
            <b>{rate}</b> frames/s
          </span>
          <span>
            <b>{rows.length}</b> buffered / {RING_CAPACITY}
          </span>
          {view.hidden > 0 && (
            <span>
              showing newest <b>{view.rendered.length}</b> of {view.shown.length}
            </span>
          )}
          {view.newer > 0 && (
            <span>
              <b>{view.newer}</b> newer
            </span>
          )}
          <span class={dropped ? 'bad' : undefined}>
            <b>{dropped}</b> dropped
          </span>
          <span>
            tap <b>{running ? 'on' : 'off'}</b>
          </span>
          <span>{follow ? 'following' : 'held'}</span>
          {connection.value !== 'live' && <span class="bad">socket {connection.value}</span>}
        </div>
      </Card>
    </>
  )
}

function IdleNote() {
  return (
    <div class="snf-idle">
      The sniffer is off. Nothing is captured and nothing is sent until you start it.
    </div>
  )
}

function FrameLog({
  rendered,
  hidden,
  logRef,
  onScroll,
}: {
  rendered: RenderedRow[]
  hidden: number
  logRef: { current: HTMLDivElement | null }
  onScroll: (event: Event) => void
}) {
  if (!rendered.length) {
    return <div class="snf-idle">Listening — no frames yet.</div>
  }
  return (
    <div class="snf-log" ref={logRef} onScroll={onScroll}>
      <div class="snf-row snf-head">
        <span>Time</span>
        <span>+ms</span>
        <span />
        <span>Hex</span>
        <span>Command</span>
        <span>Target</span>
      </div>
      {hidden > 0 && (
        <div class="snf-trunc">
          {hidden.toLocaleString()} older matching frame(s) held back from the view — still
          buffered, and Copy takes all of them
        </div>
      )}
      {rendered.map(({ row, delta }) =>
        row.kind === 'gap' ? (
          <div class="snf-gap" key={row.seq}>
            {row.lost} frame(s) dropped — the tap could not keep up
          </div>
        ) : (
          <FrameRow row={row} delta={delta} key={row.seq} />
        ),
      )}
    </div>
  )
}

function FrameRow({ row, delta }: { row: FrameRowData; delta: number | null }) {
  const f = row.frame
  return (
    <div class={`snf-row${f.attempt > 0 ? ' retry' : ''}`}>
      <span class="snf-t" title={row.anchored ? undefined : UNANCHORED_CLOCK_HINT}>
        {row.time}
      </span>
      <span class="snf-d">{delta === null ? '—' : delta}</span>
      <span class={`snf-dir ${f.dir}`}>{DIR_GLYPH[f.dir] ?? '?'}</span>
      <span class="snf-hex">{f.hex}</span>
      <span class={`snf-name${f.name.startsWith('COMMAND 0x') || f.name.startsWith('EXTENDED 0x') ? ' unknown' : ''}`}>
        {f.name}
        {f.is_query && <span class="snf-q"> ?</span>}
        {f.detail && <span class="snf-detail"> {f.detail}</span>}
        {f.attempt > 0 && <span class="snf-retry"> retry {f.attempt}</span>}
      </span>
      <span class="snf-tgt">{row.target}</span>
    </div>
  )
}

function frameRow(frame: SnifferFrame, seq: number): FrameRowData {
  const target = targetLabel(frame.target)
  const clock = deviceClock(frame.ts_ms)
  return {
    kind: 'frame',
    seq,
    frame,
    time: clock.text,
    anchored: clock.anchored,
    target,
    hay: `${frame.hex} ${frame.name} ${frame.detail ?? ''} ${target}`.toLowerCase(),
  }
}

function appendBatch(prev: Row[], batch: SnifferBatch, seq: { current: number }): Row[] {
  const added: Row[] = []
  if (batch.dropped_since > 0) {
    seq.current += 1
    added.push({ kind: 'gap', seq: seq.current, lost: batch.dropped_since })
  }
  for (const frame of batch.frames) {
    seq.current += 1
    added.push(frameRow(frame, seq.current))
  }
  if (!added.length) return prev
  const tail = added.length > RING_CAPACITY ? added.slice(added.length - RING_CAPACITY) : added
  const keep = RING_CAPACITY - tail.length
  const head = prev.length > keep ? prev.slice(prev.length - keep) : prev
  return head.concat(tail)
}

function matches(row: Row, dir: DirFilter, hideQueries: boolean, needle: string): boolean {
  if (row.kind === 'gap') return true
  const f = row.frame
  if (dir !== 'all' && f.dir !== dir) return false
  if (hideQueries && f.is_query) return false
  if (!needle) return true
  return row.hay.includes(needle)
}
