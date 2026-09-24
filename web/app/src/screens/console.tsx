import { useState } from 'preact/hooks'
import { api } from '../api/client'
import type {
  CommissioningStepName,
  CommissioningStepRequest,
  DaliCommandResponse,
} from '../api/types'
import { Badge, Chip } from '../components/ui'
import { ADAPTER, hex2, hex4, timestamp } from '../format'
import { errorMessage } from '../toast'

type Tab = 'command' | 'level' | 'raw'

interface LogEntry {
  ts: string
  kind: 'tx' | 'rx' | 'noans' | 'err'
  bytes: string
  desc: string
}

function errText(r: DaliCommandResponse): string | null {
  if (r.error == null) return null
  return r.message ?? r.error_code ?? r.error
}

function parseNum(s: string): number | null {
  const t = s.trim()
  if (t === '') return null
  const n = t.toLowerCase().startsWith('0x') ? Number.parseInt(t, 16) : Number.parseInt(t, 10)
  return Number.isFinite(n) ? n : null
}

const PRESETS: {
  label: string
  tab: Tab
  fields: Partial<Record<'wire' | 'cmd' | 'repeat' | 'level' | 'frame', string>>
}[] = [
  { label: 'Query Status', tab: 'command', fields: { wire: '0x01', cmd: '0x90', repeat: '1' } },
  { label: 'Off', tab: 'command', fields: { wire: '0x01', cmd: '0x00', repeat: '1' } },
  { label: 'DAPC 254', tab: 'level', fields: { wire: '0x00', level: '254' } },
  { label: 'Query Device Type', tab: 'command', fields: { wire: '0x01', cmd: '0x99', repeat: '1' } },
]


const COMMISSIONING_STEPS: CommissioningStepName[] = [
  'initialise',
  'randomise',
  'search-address',
  'compare',
  'program-short-address',
  'verify-short-address',
  'query-short-address',
  'withdraw',
  'terminate',
  'physical-selection',
]

const STEPS_WITH_SHORT = new Set<CommissioningStepName>([
  'program-short-address',
  'verify-short-address',
])

function CommissioningStepsPanel({ onLog }: { onLog: (entries: LogEntry[]) => void }) {
  const [open, setOpen] = useState(false)
  const [step, setStep] = useState<CommissioningStepName>('initialise')
  const [scope, setScope] = useState<'all' | 'unaddressed' | 'short'>('all')
  const [short, setShort] = useState('0')
  const [search, setSearch] = useState('0xFFFFFF')
  const [busy, setBusy] = useState(false)

  const needsShort = STEPS_WITH_SHORT.has(step) || (step === 'initialise' && scope === 'short')
  const needsSearch = step === 'search-address'

  const send = async () => {
    const body: CommissioningStepRequest = {}
    if (step === 'initialise') body.scope = scope
    if (needsShort) {
      const v = parseNum(short)
      if (v == null) return
      body.short_address = v
    }
    if (needsSearch) {
      const v = parseNum(search)
      if (v == null) return
      body.search_address = v
    }
    setBusy(true)
    onLog([{ ts: timestamp(), kind: 'tx', bytes: step, desc: JSON.stringify(body) }])
    try {
      const r = await api.commissioningStep(ADAPTER, step, body)
      const bits: string[] = []
      if (r.match != null) bits.push(`match=${r.match}`)
      if (r.answer != null) {
        bits.push(
          r.answer === 'address' ? `short=${r.short_address}` : `answer=${r.answer}`,
        )
      } else if (r.short_address !== undefined) {
        bits.push(`short=${r.short_address == null ? 'none' : r.short_address}`)
      }
      if (r.backward_violation) bits.push('violating frame (§8.2.5)')
      onLog([
        {
          ts: timestamp(),
          kind: r.success ? 'rx' : 'err',
          bytes: r.backward_frame == null ? '—' : hex2(r.backward_frame),
          desc: bits.length ? bits.join(' · ') : r.success ? 'ok' : 'failed',
        },
      ])
    } catch (e) {
      onLog([{ ts: timestamp(), kind: 'err', bytes: '—', desc: errorMessage(e) }])
    } finally {
      setBusy(false)
    }
  }

  return (
    <div class="comm-steps">
      <button class="comm-steps-toggle" onClick={() => setOpen((o) => !o)}>
        {open ? '▾' : '▸'} Commissioning steps
      </button>
      {open && (
        <div class="comm-steps-body">
          <select
            class="sel sm"
            value={step}
            disabled={busy}
            onChange={(e) => setStep((e.target as HTMLSelectElement).value as CommissioningStepName)}
          >
            {COMMISSIONING_STEPS.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
          {step === 'initialise' && (
            <select
              class="sel sm"
              value={scope}
              disabled={busy}
              onChange={(e) =>
                setScope((e.target as HTMLSelectElement).value as 'all' | 'unaddressed' | 'short')
              }
            >
              <option value="all">all</option>
              <option value="unaddressed">unaddressed</option>
              <option value="short">short</option>
            </select>
          )}
          {needsShort && (
            <input
              class="mono"
              value={short}
              disabled={busy}
              onInput={(e) => setShort((e.target as HTMLInputElement).value)}
              placeholder="short 0..63"
            />
          )}
          {needsSearch && (
            <input
              class="mono"
              value={search}
              disabled={busy}
              onInput={(e) => setSearch((e.target as HTMLInputElement).value)}
              placeholder="24-bit search address"
            />
          )}
          <button class="btn sm" disabled={busy} onClick={send}>
            {busy ? 'sending…' : 'Send step'}
          </button>
        </div>
      )}
    </div>
  )
}

export function Console() {
  const [tab, setTab] = useState<Tab>('command')
  const [wire, setWire] = useState('0x01')
  const [cmd, setCmd] = useState('0x90')
  const [repeat, setRepeat] = useState('1')
  const [level, setLevel] = useState('254')
  const [frame, setFrame] = useState('0x0190')
  const [expectsBackward, setExpectsBackward] = useState(false)
  const [log, setLog] = useState<LogEntry[]>([])
  const [busy, setBusy] = useState(false)

  const append = (entries: LogEntry[]) => setLog((prev) => [...entries, ...prev])

  const logResponse = (r: DaliCommandResponse) => {
    if (r.success && r.error == null) {
      append([{ ts: timestamp(), kind: 'rx', bytes: hex2(r.backward_frame), desc: 'answer' }])
    } else if (r.error === 'timeout') {
      append([{ ts: timestamp(), kind: 'noans', bytes: '—', desc: 'no answer (timeout)' }])
    } else {
      append([{ ts: timestamp(), kind: 'err', bytes: '—', desc: errText(r) ?? 'error' }])
    }
  }

  const send = async () => {
    setBusy(true)
    try {
      if (tab === 'command') {
        const w = parseNum(wire)
        const c = parseNum(cmd)
        const rc = parseNum(repeat) ?? 1
        if (w == null || c == null) {
          append([{ ts: timestamp(), kind: 'err', bytes: '—', desc: 'invalid wire_address / command' }])
          return
        }
        append([
          {
            ts: timestamp(),
            kind: 'tx',
            bytes: `${hex2(w)} ${hex2(c)}`,
            desc: `command · repeat ${rc}`,
          },
        ])
        logResponse(await api.daliCommand({ wire_address: w, command: c, repeat_count: rc }))
      } else if (tab === 'level') {
        const w = parseNum(wire)
        const lv = parseNum(level)
        if (w == null || lv == null) {
          append([{ ts: timestamp(), kind: 'err', bytes: '—', desc: 'invalid wire_address / level' }])
          return
        }
        append([
          { ts: timestamp(), kind: 'tx', bytes: `${hex2(w)} ${hex2(lv)}`, desc: `DAPC ${lv}` },
        ])
        logResponse(await api.daliLevel({ wire_address: w, level: lv }))
      } else {
        const f = parseNum(frame)
        if (f == null) {
          append([{ ts: timestamp(), kind: 'err', bytes: '—', desc: 'invalid frame' }])
          return
        }
        append([
          {
            ts: timestamp(),
            kind: 'tx',
            bytes: hex4(f),
            desc: `raw${expectsBackward ? ' · expects backward' : ''}`,
          },
        ])
        logResponse(await api.daliRaw({ frame: f, expects_backward: expectsBackward }))
      }
    } catch (e) {
      append([{ ts: timestamp(), kind: 'err', bytes: '—', desc: errorMessage(e) }])
    } finally {
      setBusy(false)
    }
  }

  const applyPreset = (p: (typeof PRESETS)[number]) => {
    setTab(p.tab)
    if (p.fields.wire !== undefined) setWire(p.fields.wire)
    if (p.fields.cmd !== undefined) setCmd(p.fields.cmd)
    if (p.fields.repeat !== undefined) setRepeat(p.fields.repeat)
    if (p.fields.level !== undefined) setLevel(p.fields.level)
    if (p.fields.frame !== undefined) setFrame(p.fields.frame)
  }

  const endpoint =
    tab === 'command' ? '/api/v1/dali/command' : tab === 'level' ? '/api/v1/dali/level' : '/api/v1/dali/raw'

  return (
    <>
      <div class="crumbs">
        <a href="#/">Adapter 0</a> / DALI console
      </div>
      <div class="head">
        <h1>DALI console</h1>
        <Badge>diagnostic</Badge>
        <span class="spacer" />
        <span class="sub">Adapter 0</span>
      </div>

      <div class="panes console">
        <div class="card">
          <header>
            <h3>Send frame</h3>
          </header>
          <div class="tabs">
            {(['command', 'level', 'raw'] as Tab[]).map((t) => (
              <button key={t} class={`tab${tab === t ? ' active' : ''}`} onClick={() => setTab(t)}>
                {t.charAt(0).toUpperCase() + t.slice(1)}
              </button>
            ))}
          </div>
          <div class="tabline" />
          <div class="form">
            {tab !== 'raw' && (
              <div class="field">
                <label>wire_address</label>
                <input type="text" value={wire} onInput={(e) => setWire(e.currentTarget.value)} />
                <div class="aid">short address byte · indirect commands set bit 0 = 1</div>
              </div>
            )}
            {tab === 'command' && (
              <div class="row2">
                <div class="field">
                  <label>command</label>
                  <input type="text" value={cmd} onInput={(e) => setCmd(e.currentTarget.value)} />
                  <div class="aid">opcode byte</div>
                </div>
                <div class="field">
                  <label>repeat_count</label>
                  <input type="text" value={repeat} onInput={(e) => setRepeat(e.currentTarget.value)} />
                  <div class="aid">send-twice = 2</div>
                </div>
              </div>
            )}
            {tab === 'level' && (
              <div class="field">
                <label>level</label>
                <input type="text" value={level} onInput={(e) => setLevel(e.currentTarget.value)} />
                <div class="aid">DAPC arc power 0–254</div>
              </div>
            )}
            {tab === 'raw' && (
              <>
                <div class="field">
                  <label>frame</label>
                  <input type="text" value={frame} onInput={(e) => setFrame(e.currentTarget.value)} />
                  <div class="aid">16-bit forward frame, hex or decimal</div>
                </div>
                <label class="checkline">
                  <input
                    type="checkbox"
                    checked={expectsBackward}
                    onChange={(e) => setExpectsBackward(e.currentTarget.checked)}
                  />
                  expects_backward
                </label>
              </>
            )}
            <div class="sendrow">
              <button class="btn primary" onClick={send} disabled={busy}>
                Send
              </button>
              <span class="ep">POST {endpoint} · JSON</span>
            </div>
          </div>
          <div class="hintbox">
            Diagnostic path — sends raw 16-bit frames and bypasses product semantics. Registry
            runtime state is not updated by replies here.
          </div>
          <div class="presets">
            <span class="lbl">Presets</span>
            {PRESETS.map((p) => (
              <button key={p.label} class="btn preset" onClick={() => applyPreset(p)}>
                {p.label}
              </button>
            ))}
          </div>
        </div>

        <div class="card">
          <header>
            <h3>Exchange log</h3>
            <button class="act" onClick={() => setLog([])}>
              Clear
            </button>
          </header>
          <div class="log">
            {log.length === 0 && <div class="empty">No frames sent yet.</div>}
            {log.map((l, i) => (
              <div key={i} class={`lrow${l.kind !== 'tx' ? ` ${l.kind}` : ''}`}>
                <span class="ts">{l.ts}</span>
                <span class={`dir ${l.kind === 'tx' ? 'tx' : l.kind === 'rx' ? 'rx' : l.kind === 'err' ? 'err' : 'tx'}`}>
                  {l.kind === 'tx' ? '→' : l.kind === 'rx' ? '←' : l.kind === 'err' ? '✕' : '·'}
                </span>
                <span class="bytes">{l.bytes}</span>
                <span class="desc">{l.desc}</span>
                <span class="lat" />
              </div>
            ))}
          </div>
        </div>
      </div>
      <CommissioningStepsPanel onLog={append} />
      {busy && <Chip cls="run">sending…</Chip>}
    </>
  )
}
