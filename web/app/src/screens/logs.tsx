import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'preact/hooks'

import { connection, setLogLevel, subscribe, type LogLevel, type WsEvent } from '../api/ws'
import { deviceClock, UNANCHORED_CLOCK_HINT } from '../format'
import { Card } from '../components/ui'
import { anchorIndex, isAtBottom, windowBounds } from './follow-scroll'

const RING_CAPACITY = 2000

const RENDER_WINDOW = 400

const LEVELS: LogLevel[] = ['error', 'warn', 'info', 'debug']

interface LogLine {
  ts_ms: number
  seq: number
  level: LogLevel
  target: string
  text: string
}

interface LogBatch {
  dropped_since: number
  lines: LogLine[]
}

type Row =
  | { kind: 'line'; seq: number; line: LogLine; time: string; anchored: boolean; hay: string }
  | { kind: 'gap'; seq: number; lost: number }

function lineRow(line: LogLine): Row {
  const clock = deviceClock(line.ts_ms)
  return {
    kind: 'line',
    seq: line.seq,
    line,
    time: clock.text,
    anchored: clock.anchored,
    hay: `${line.target} ${line.text}`.toLowerCase(),
  }
}

function appendBatch(prev: Row[], batch: LogBatch, gapSeq: { current: number }): Row[] {
  const seen = new Set(prev.filter((r) => r.kind === 'line').map((r) => r.seq))
  const added: Row[] = []
  if (batch.dropped_since > 0) {
    gapSeq.current -= 1
    added.push({ kind: 'gap', seq: gapSeq.current, lost: batch.dropped_since })
  }
  for (const line of batch.lines) {
    if (seen.has(line.seq)) continue
    added.push(lineRow(line))
  }
  if (!added.length) return prev
  const tail = added.length > RING_CAPACITY ? added.slice(added.length - RING_CAPACITY) : added
  const keep = RING_CAPACITY - tail.length
  const head = prev.length > keep ? prev.slice(prev.length - keep) : prev
  return head.concat(tail)
}

function matches(row: Row, level: LogLevel, needle: string): boolean {
  if (row.kind === 'gap') return true
  if (LEVELS.indexOf(row.line.level) > LEVELS.indexOf(level)) return false
  return !needle || row.hay.includes(needle)
}

export function Logs() {
  const [running, setRunning] = useState(false)
  const [level, setLevel] = useState<LogLevel>('warn')
  const [needle, setNeedle] = useState('')
  const [rows, setRows] = useState<Row[]>([])
  const [dropped, setDropped] = useState(0)
  const [follow, setFollow] = useState(true)
  const listRef = useRef<HTMLDivElement | null>(null)
  const anchor = useRef<number | null>(null)
  const gapSeq = useRef(0)

  useEffect(() => {
    if (!running) return undefined
    setLogLevel(level)
    const dispose = subscribe(['logs'], (event: WsEvent) => {
      const frame = event as unknown as Record<string, unknown>
      if (frame.type === 'DropNotice') return
      if (event.type !== 'LogBatch') return
      const batch = event.payload as LogBatch
      if (batch.dropped_since > 0) setDropped((d) => d + batch.dropped_since)
      setRows((prev) => appendBatch(prev, batch, gapSeq))
    })
    return dispose
  }, [running])

  useEffect(() => {
    if (running) setLogLevel(level)
  }, [level, running])

  const view = useMemo(() => {
    const needleLc = needle.trim().toLowerCase()
    const shown = rows.filter((row) => matches(row, level, needleLc))
    const held = anchorIndex(shown.map((row) => row.seq), anchor.current)
    const { start, end, newer } = windowBounds(shown.length, held, RENDER_WINDOW)
    return { shown, rendered: shown.slice(start, end), hidden: start, newer }
  }, [rows, level, needle, follow])

  useLayoutEffect(() => {
    if (!follow) return
    const el = listRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [view.rendered, follow])

  const newestSeq = () => view.shown[view.shown.length - 1]?.seq ?? null

  const setFollowing = (on: boolean) => {
    anchor.current = on ? null : newestSeq()
    setFollow(on)
  }

  const onScroll = (event: Event) => {
    const el = event.currentTarget as HTMLDivElement
    const atBottom = isAtBottom(el.scrollHeight, el.scrollTop, el.clientHeight)
    if (atBottom) anchor.current = null
    else if (anchor.current === null) anchor.current = newestSeq()
    setFollow(atBottom)
  }

  return (
    <>
      <div class="crumbs">
        <a href="#/">System</a> / Log
      </div>
      <div class="head">
        <h1>Firmware log</h1>
        <span class="spacer" />
        <span class="sub">{running ? 'streaming' : 'stopped'}</span>
      </div>
      <p class="hint">
        What the controller is saying, live — including the ESP-IDF components that never reach a
        Rust log macro. The controller keeps a small warn-level ring even with this screen closed,
        so subscribing shows you what happened just before you looked.
      </p>
      <Card title="Log stream">
        <div class="log-bar">
          <button
            class={running ? 'btn stop' : 'btn primary'}
            onClick={() => setRunning(!running)}
          >
            {running ? 'Stop' : 'Start'}
          </button>
          <button
            class="btn ghost"
            onClick={() => {
              setRows([])
              setDropped(0)
              setFollowing(true)
            }}
          >
            Clear
          </button>
          <div class="log-sep" />
          <div class="log-levels">
            {LEVELS.map((option) => (
              <button class={level === option ? 'on' : undefined} onClick={() => setLevel(option)}>
                {option[0].toUpperCase() + option.slice(1)}
              </button>
            ))}
          </div>
          <div class="log-levels">
            <button class={follow ? 'on' : undefined} onClick={() => setFollowing(true)}>
              Follow
            </button>
            <button class={follow ? undefined : 'on'} onClick={() => setFollowing(false)}>
              Manual
            </button>
          </div>
          <input
            class="log-search"
            placeholder="module / text"
            value={needle}
            onInput={(e) => setNeedle((e.target as HTMLInputElement).value)}
          />
        </div>
        {running ? (
          <LogList rendered={view.rendered} hidden={view.hidden} listRef={listRef} onScroll={onScroll} />
        ) : (
          <div class="log-idle">
            The log is off. The controller keeps a small warn-level ring; nothing is sent until you
            start.
          </div>
        )}
        <div class="log-foot">
          <span>
            level <b>{level}</b>
          </span>
          <span>
            <b>{rows.length}</b> buffered / {RING_CAPACITY}
          </span>
          <span class={dropped ? 'bad' : undefined}>
            <b>{dropped}</b> dropped
          </span>
          {view.newer > 0 && (
            <span>
              <b>{view.newer}</b> newer
            </span>
          )}
          <span>{follow ? 'following' : 'held'}</span>
          {connection.value !== 'live' && <span class="bad">socket {connection.value}</span>}
        </div>
      </Card>
    </>
  )
}

function LogList({
  rendered,
  hidden,
  listRef,
  onScroll,
}: {
  rendered: Row[]
  hidden: number
  listRef: { current: HTMLDivElement | null }
  onScroll: (event: Event) => void
}) {
  if (!rendered.length) {
    return <div class="log-idle">Listening — nothing at this level yet.</div>
  }
  return (
    <div class="log-list" ref={listRef} onScroll={onScroll}>
      {hidden > 0 && (
        <div class="log-replay">
          {hidden.toLocaleString()} older line(s) held back from the view — still buffered
        </div>
      )}
      {rendered.map((row) =>
        row.kind === 'gap' ? (
          <div class="log-gap" key={row.seq}>
            {row.lost} line(s) dropped — the ring filled faster than the socket drained it
          </div>
        ) : (
          <LogRow row={row} key={row.seq} />
        ),
      )}
    </div>
  )
}

function LogRow({ row }: { row: Extract<Row, { kind: 'line' }> }) {
  const { line } = row
  return (
    <div class={`log-row${line.level === 'error' ? ' error' : ''}`}>
      <span class="log-t" title={row.anchored ? undefined : UNANCHORED_CLOCK_HINT}>
        {row.time}
      </span>
      <span class={`log-lvl ${line.level}`}>{line.level}</span>
      <span class="log-tag">{line.target}</span>
      <span class="log-msg">{line.text}</span>
    </div>
  )
}
