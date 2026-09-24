import { useEffect, useRef, useState } from 'preact/hooks'

import { api, ApiError, isRulesParseError } from '../api/client'
import type { RuleJson, RulesDocument, RulesDocumentJson, RulesParseResult } from '../api/types'
import { connection, subscribe, type WsChannel, type WsEvent } from '../api/ws'
import { timestamp } from '../format'
import { usePoll } from '../hooks'
import { errorMessage, mutate, notify, opCommitted, trackOp } from '../toast'

const MAX_RULES_SOURCE_BYTES = 12_240

const PARSE_DEBOUNCE_MS = 500

const LINE_HEIGHT_PX = 20
const EDITOR_PAD_PX = 12

const FEED_CAPACITY = 100

const FEED_CHANNELS: WsChannel[] = ['input', 'rules']

const UTF8 = new TextEncoder()

const SNIPPETS: { label: string; code: string }[] = [
  {
    label: 'toggle button',
    code: `rule "коридор: вкл/выкл" {
  when input(dev=3, inst=0) is short_press
  do   lamp("коридор").toggle()
}

rule "коридор: диммирование" {
  when input(dev=3, inst=0) is long_press_repeat
  do   lamp("коридор").dim_hold(+60)      # 60 шагов в секунду удержания
}

rule "коридор: полный свет" {
  when input(dev=3, inst=0) is double_press
  do   lamp("коридор").on(level=254)
}`,
  },
  {
    label: 'night light',
    code: `def "ночной контур" {
  scene(13).recall(group("ночь"))         # 30/254 и 2200 K одним кадром
  timer("ночь-выкл").cancel()
}

rule "ночь: вход" {
  when input(dev=5, inst=0) becomes occupied
  when input(dev=3, inst=0) is short_press
  if   time in 23:00 .. sunrise
  do   call("ночной контур")
}

rule "ночь: погашение" {
  when input(dev=5, inst=0) becomes vacant
  if   time in 23:00 .. sunrise
  do   group("ночь").level(10)   # притухли: «сейчас выключусь»
       timer("ночь-выкл").start(30s)
}

rule "ночь: выключение" {
  when timer("ночь-выкл") fires
  do   group("ночь").off()
       hcl.resume(group("ночь"))
}`,
  },
  {
    label: 'staircase timer',
    code: `rule "лестница: свет по кнопке" {
  when input(dev=3, inst=0) is short_press
  do   lamp("лестница").on(level=200)
       timer("лестница-выкл").restart(2m)   # повторное нажатие продлевает
}

rule "лестница: автовыключение" {
  when timer("лестница-выкл") fires
  do   lamp("лестница").off()
}`,
  },
  {
    label: 'away',
    code: `rule "ушёл" {
  when input(dev=3, inst=2) is long_press_start
  do   broadcast.off()
       var("режим").set("нет дома")
       mqtt.publish("dali2rust/mode", "away", retain=true)
       after 5m do { hcl.resume(broadcast) }
}`,
  },
  {
    label: 'scene panel + LEDs',
    code: `rule "зал: сцены с панели" {
  when input(group=4, type=button) is short_press
  do   scene(event.option).recall(group("зал"))
       panel_select(group=4, selected=event.option)
}

rule "зал: вернуть индикацию" {
  when input device(dev=3) power cycled
  do   panel_select(group=4, selected=var("зал-сцена"))
}`,
  },
]

function ruleSummary(r: RuleJson): string {
  const n = (k: number, word: string) => `${k} ${word}${k === 1 ? '' : 's'}`
  const parts = [n(r.triggers.length, 'trigger')]
  if (r.conditions.length > 0) parts.push(n(r.conditions.length, 'condition'))
  parts.push(n(r.actions.length, 'action'))
  if (r.cooldown_ms > 0) parts.push(`cooldown ${r.cooldown_ms / 1000} s`)
  if (r.hold_hcl) parts.push('holds HCL')
  return parts.join(' · ')
}

interface InputFeedPayload {
  scheme?: number | null
  short_address?: number | null
  instance_number?: number | null
  event?: string | null
  event_info?: number | null
}

interface RuleActivationPayload {
  rule_name?: string | null
  dry?: boolean | null
  effects?: number | null
  partial?: number | null
  trigger_to_publish_ms?: number | null
}

interface FeedRow {
  seq: number
  at: string
  short: number | null
  instance: number | null
  event: string | null
  scheme: number | null
  info: number | null
  lifecycle: boolean
  atMs: number
  rule?: string | null
  partial?: number
  ms?: number | null
}

const PARTIAL_REASONS: Record<number, string> = {
  1: 'partial — condition',
  2: 'partial — effect budget',
  3: 'partial — chain depth',
}

const RULE_SETTLE_MS = 1000

function ruleSpans(source: string): Map<string, { from: number; to: number }> {
  const spans = new Map<string, { from: number; to: number }>()
  const lines = source.split('\n')
  let name: string | null = null
  let depth = 0
  let from = 0
  lines.forEach((line, i) => {
    if (name === null) {
      const m = /^\s*rule\s+"([^"]+)"\s*\{/.exec(line)
      if (m) {
        name = m[1]
        from = i
        depth = 0
      }
    }
    if (name === null) return
    for (const ch of line) {
      if (ch === '{') depth += 1
      else if (ch === '}') depth -= 1
    }
    if (depth <= 0) {
      spans.set(name, { from, to: i })
      name = null
    }
  })
  return spans
}

function spliceRule(source: string, span: { from: number; to: number }, text: string): string {
  const lines = source.split('\n')
  return [...lines.slice(0, span.from), ...text.split('\n'), ...lines.slice(span.to + 1)].join('\n')
}

export function RulesScreen() {
  const doc = usePoll(async (): Promise<{ text: RulesDocument; json: RulesDocumentJson }> => {
    const [text, json] = await Promise.all([api.rules(), api.rulesJson()])
    return { text, json }
  })
  const [draft, setDraft] = useState<string | null>(null)
  const [baseRev, setBaseRev] = useState<number | null>(null)
  const [busy, setBusy] = useState(false)
  const [refused, setRefused] = useState(false)
  const [parse, setParse] = useState<RulesParseResult | null>(null)
  const [checking, setChecking] = useState(false)
  const [scrollTop, setScrollTop] = useState(0)
  const [scoped, setScoped] = useState<string | null>(null)
  const parseSeq = useRef(0)
  const taRef = useRef<HTMLTextAreaElement | null>(null)
  const caretTouched = useRef(false)

  const parseErr = parse !== null && 'error' in parse ? parse : null

  useEffect(() => {
    if (draft === null) {
      setParse(null)
      setChecking(false)
      return
    }
    setChecking(true)
    const seq = ++parseSeq.current
    const id = setTimeout(() => {
      void api
        .parseRules(draft)
        .then((r) => {
          if (parseSeq.current === seq) {
            setParse(r)
            setChecking(false)
          }
        })
        .catch(() => {
          if (parseSeq.current === seq) setChecking(false)
        })
    }, PARSE_DEBOUNCE_MS)
    return () => clearTimeout(id)
  }, [draft])

  const errLine = parseErr?.line ?? null
  useEffect(() => {
    const ta = taRef.current
    if (errLine === null || !ta) return
    const y = EDITOR_PAD_PX + (errLine - 1) * LINE_HEIGHT_PX
    if (y < ta.scrollTop || y > ta.scrollTop + ta.clientHeight - LINE_HEIGHT_PX * 2) {
      ta.scrollTop = Math.max(0, y - ta.clientHeight / 2)
      setScrollTop(ta.scrollTop)
    }
  }, [errLine])

  if (!doc.data) return <div class="empty">Loading rules…</div>
  const data = doc.data
  const text = data.text
  const rules = data.json.rules?.rules ?? null
  const source = text.source
  const full = draft ?? source
  const spans = ruleSpans(full)
  const span = scoped === null ? null : spans.get(scoped) ?? null
  const shown =
    span === null ? full : full.split('\n').slice(span.from, span.to + 1).join('\n')
  const dirty = draft !== null && draft !== source
  const drift = draft !== null && baseRev !== null && text.revision > baseRev
  const bytes = UTF8.encode(full).length

  const editDraft = (value: string) => {
    if (draft === null) setBaseRev(data.text.revision)
    setDraft(span === null ? value : spliceRule(full, span, value))
  }

  const discard = () => {
    setDraft(null)
    setBaseRev(null)
    setRefused(false)
  }

  const check = () => {
    setChecking(true)
    const seq = ++parseSeq.current
    void api
      .parseRules(full)
      .then((r) => {
        if (parseSeq.current === seq) {
          setParse(r)
          setChecking(false)
        }
      })
      .catch((e) => {
        if (parseSeq.current === seq) {
          setChecking(false)
          notify('Rules parse', 'failed', errorMessage(e))
        }
      })
  }

  const save = async () => {
    if (draft === null) return
    const rev = baseRev ?? text.revision
    setBusy(true)
    try {
      const op = await trackOp('Rules document', await api.putRules(draft, rev))
      if (!opCommitted(op)) {
        if (op?.error?.code === 'rule_set_conflict' || op?.error?.message === 'rule_set_conflict') {
          setRefused(true)
        }
        return
      }
      setDraft(null)
      setBaseRev(null)
      setRefused(false)
      doc.reload()
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        setRefused(true)
        return
      }
      if (e instanceof ApiError && e.code === 'parse_error' && isRulesParseError(e.body)) {
        setParse(e.body)
        notify('Rules document', 'failed', `parse error at ${e.body.line}:${e.body.column}`)
        return
      }
      notify('Rules document', 'failed', errorMessage(e))
    } finally {
      setBusy(false)
    }
  }

  const reloadTheirs = () => {
    discard()
    setParse(null)
    doc.reload()
  }

  const keepMine = async () => {
    try {
      const fresh = await api.rules()
      setBaseRev(fresh.revision)
      setRefused(false)
      notify('Rules document', 'info', `re-armed on revision ${fresh.revision} — Save again to overwrite`)
    } catch (e) {
      notify('Rules document', 'failed', errorMessage(e))
    }
  }

  const toggleRule = (r: RuleJson) =>
    void mutate(`Rule "${r.name}"`, () => api.patchRule(r.name, !r.enabled), doc.reload)

  const insertSnippet = (code: string) => {
    const current = draft ?? source
    const ta = taRef.current
    const at = caretTouched.current && ta ? ta.selectionStart : current.length
    const before = current.slice(0, at)
    const after = current.slice(at)
    const lead = before === '' || before.endsWith('\n\n') ? '' : before.endsWith('\n') ? '\n' : '\n\n'
    const tail = after === '' ? '\n' : after.startsWith('\n') ? '\n' : '\n\n'
    editDraft(before + lead + code.trim() + tail + after)
  }

  const prefill = (row: FeedRow) => {
    const short = row.short
    if (short === null) return
    const inst = row.instance ?? 0
    const ev = row.event ?? 'short_press'
    const verb = ev === 'occupied' || ev === 'vacant' ? 'becomes' : 'is'
    const skeleton = row.lifecycle
      ? [
          '# from the live feed',
          `rule "dev ${short}: power cycled" {`,
          `  when input device(dev=${short}) power cycled`,
          `  do   log("TODO")`,
          '}',
          '',
        ].join('\n')
      : [
          '# from the live feed',
          `rule "dev ${short} / inst ${inst}: ${ev}" {`,
          `  when input(dev=${short}, inst=${inst}) ${verb} ${ev}`,
          `  do   log("TODO")`,
          '}',
          '',
        ].join('\n')
    const current = draft ?? source
    editDraft(current === '' ? skeleton : `${skeleton}\n${current}`)
    if (taRef.current) taRef.current.scrollTop = 0
  }

  const state = checking
    ? { cls: '', label: '… checking' }
    : parse !== null
      ? 'ok' in parse
        ? { cls: 'state-ok', label: `✓ ok · ${parse.rules.length} rule${parse.rules.length === 1 ? '' : 's'}` }
        : { cls: 'state-err', label: '✗ parse error' }
      : text.diagnostic
        ? { cls: 'state-err', label: '✗ document inactive' }
        : { cls: 'state-ok', label: `✓ ${text.rule_count} rule${text.rule_count === 1 ? '' : 's'}` }

  return (
    <div class="rules">
      <div class="head">
        <h1>Rules</h1>
        <span class="spacer" />
        <button class="btn" disabled={busy} onClick={check}>
          Check
        </button>
        <button
          class="btn"
          disabled={busy || parse === null || 'error' in parse || parse.rules.length === 0}
          title="Evaluate the first parsed rule on the device without executing anything"
          onClick={() => {
            const name = parse !== null && !('error' in parse) ? parse.rules[0]?.name : undefined
            if (name) void mutate('Dry run', () => api.runRule(name, true), doc.reload)
          }}
        >
          Dry-run
        </button>
        {draft !== null && (
          <button class="btn ghost" disabled={busy} onClick={discard}>
            Cancel
          </button>
        )}
        <button class="btn primary" disabled={busy || !dirty} onClick={() => void save()}>
          Save
        </button>
      </div>
      <p class="sub">
        One document, plain text — the device stores your bytes, comments included. Validation
        runs on the device's own parser; there is no second grammar in the browser. Dry-run
        evaluates on the device and executes nothing; the live feed below shows what fires.
      </p>

      {text.diagnostic && (
        <div class="warnbar">
          <b>Stored document is not active:</b> {text.diagnostic}
        </div>
      )}

      {(refused || drift) && (
        <div class="conflict">
          <b>Document changed on the device.</b>{' '}
          {refused
            ? 'The save was refused (409 rule_set_conflict) — your draft was built on an older revision.'
            : `Someone saved revision ${text.revision} while you edit revision ${baseRev} — saving now would be refused (409).`}
          <div class="acts">
            <button class="btn sm" disabled={busy} onClick={reloadTheirs}>
              Reload theirs
            </button>
            <button class="btn sm ghost" disabled={busy} onClick={() => void keepMine()}>
              Keep mine
            </button>
            <span class="d">
              Reload replaces your draft; Keep mine re-arms Save against the fresh revision —
              the overwrite still needs a second explicit Save.
            </span>
          </div>
        </div>
      )}

      <div class="cols">
        <div class="editor">
          <div class="tabs">
            <span
              class={scoped === null ? 't on' : 't'}
              onClick={() => setScoped(null)}
            >
              Whole document
            </span>
            {scoped !== null && <span class="t on">{scoped}</span>}
          </div>
          <div class="bar">
            <span class={state.cls}>{state.label}</span>
            <span>·</span>
            <span>revision {text.revision}</span>
            <span>·</span>
            <span class={bytes > MAX_RULES_SOURCE_BYTES ? 'over' : undefined}>
              {bytes.toLocaleString()} B of {MAX_RULES_SOURCE_BYTES.toLocaleString()}
            </span>
            {dirty && <span>· unsaved</span>}
          </div>
          <div class="editwrap">
            {parseErr && (
              <div
                class="errline"
                style={{ top: `${EDITOR_PAD_PX + (parseErr.line - 1) * LINE_HEIGHT_PX - scrollTop}px` }}
              />
            )}
            <textarea
              ref={taRef}
              class="code"
              spellcheck={false}
              wrap="off"
              value={shown}
              onFocus={() => {
                caretTouched.current = true
              }}
              onInput={(e) => editDraft(e.currentTarget.value)}
              onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
            />
          </div>
          {parseErr && (
            <div class="parse-msg">
              {parseErr.line}:{parseErr.column} · {parseErr.message}
            </div>
          )}
        </div>

        <div class="rcol">
          <RulesPanel
            rules={rules}
            diagnostic={text.diagnostic}
            busy={busy}
            onToggle={toggleRule}
            selected={scoped}
            onSelect={(name) => setScoped(spans.has(name) ? name : null)}
          />
          <div class="panel">
            <h2>Snippets</h2>
            <div class="snips">
              {SNIPPETS.map((s) => (
                <button class="snip" key={s.label} onClick={() => insertSnippet(s.code)}>
                  {s.label}
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>

      <LiveFeed ruleCount={text.rule_count} onPrefill={prefill} />
    </div>
  )
}

function RulesPanel({
  rules,
  diagnostic,
  busy,
  onToggle,
  selected,
  onSelect,
}: {
  rules: RuleJson[] | null
  diagnostic: string | null
  busy: boolean
  onToggle: (r: RuleJson) => void
  selected: string | null
  onSelect: (name: string) => void
}) {
  return (
    <div class="panel">
      <h2>Rules</h2>
      {rules === null ? (
        <div class="pempty">
          {diagnostic
            ? 'The stored document is not active — see the banner above.'
            : 'No typed projection from the device yet.'}
        </div>
      ) : rules.length === 0 ? (
        <div class="pempty">No rules yet — write one in the editor, or start from a snippet.</div>
      ) : (
        rules.map((r) => (
          <div
            class={selected === r.name ? 'rrow on' : 'rrow'}
            key={r.name}
            onClick={() => onSelect(r.name)}
          >
            <button
              class={r.enabled ? 'toggle on' : 'toggle'}
              disabled={busy}
              aria-label={r.enabled ? `Disable "${r.name}"` : `Enable "${r.name}"`}
              onClick={() => onToggle(r)}
            >
              <i />
            </button>
            <span class={r.enabled ? 'n' : 'n dis'} title={`Edit "${r.name}"`}>
              {r.name}
            </span>
            <span class="t">{ruleSummary(r)}</span>
          </div>
        ))
      )}
    </div>
  )
}

function LiveFeed({
  ruleCount,
  onPrefill,
}: {
  ruleCount: number
  onPrefill: (row: FeedRow) => void
}) {
  const [rows, setRows] = useState<FeedRow[]>([])
  const [dropped, setDropped] = useState(0)
  const seq = useRef(0)

  useEffect(() => {
    const dispose = subscribe(FEED_CHANNELS, (event: WsEvent) => {
      if (event.type === 'DropNotice') {
        const lost = (event as unknown as { dropped_count?: number }).dropped_count ?? 0
        setDropped((d) => d + lost)
        return
      }
      if (event.type === 'RulesActivationEvent') {
        const a = (event.payload ?? {}) as RuleActivationPayload
        if (a.dry) return
        setRows((prev) => {
          const now = Date.now()
          const i = prev
            .map((r) => r.rule === undefined && r.short !== null && now - r.atMs < RULE_SETTLE_MS)
            .lastIndexOf(true)
          if (i < 0) return prev
          const next = prev.slice()
          next[i] = {
            ...next[i],
            rule: a.rule_name ?? null,
            partial: a.partial ?? 0,
            ms: a.trigger_to_publish_ms ?? null,
          }
          return next
        })
        return
      }
      if (
        event.type !== 'DaliInputEventObservedEvent' &&
        event.type !== 'DaliInputDeviceLifecycleEvent'
      ) {
        return
      }
      const p = (event.payload ?? {}) as InputFeedPayload
      seq.current += 1
      const row: FeedRow = {
        seq: seq.current,
        at: timestamp(),
        short: p.short_address ?? null,
        instance: p.instance_number ?? null,
        event: p.event ?? null,
        scheme: p.scheme ?? null,
        info: p.event_info ?? null,
        lifecycle: event.type === 'DaliInputDeviceLifecycleEvent',
        atMs: Date.now(),
      }
      setRows((prev) => {
        const next = [...prev, row]
        return next.length > FEED_CAPACITY ? next.slice(next.length - FEED_CAPACITY) : next
      })
    })
    return dispose
  }, [])

  const [, setTick] = useState(0)
  const settleNow = Date.now()
  const deadlines = rows
    .filter((r) => r.rule === undefined && r.atMs + RULE_SETTLE_MS > settleNow)
    .map((r) => r.atMs + RULE_SETTLE_MS)
  const dueAt = deadlines.length > 0 ? Math.min(...deadlines) : null
  useEffect(() => {
    if (dueAt === null) return
    const t = setTimeout(() => setTick((n) => n + 1), Math.max(0, dueAt - Date.now()))
    return () => clearTimeout(t)
  }, [dueAt])

  const list = rows.slice().reverse()

  return (
    <div class="panel feed">
      <h2>
        Live input events
        {dropped > 0 && <span class="lag">dropped_since: {dropped}</span>}
      </h2>
      {ruleCount === 0 && <div class="fhint">No rules yet — every event below passes by unmatched.</div>}
      {connection.value !== 'live' && (
        <div class="fnote">push channel is {connection.value} — the feed fills only while the socket is live</div>
      )}
      <div class="fscroll">
        {list.length === 0 ? (
          <div class="pempty">Listening — press a button on the bus and its event lands here.</div>
        ) : (
          list.map((row) => <FeedLine key={row.seq} row={row} onPrefill={onPrefill} />)
        )}
      </div>
      <div class="fnote">
        Rule activations — which rule fired, outcome, duration — join this feed with the
        engine (I10-B).
      </div>
    </div>
  )
}

function FeedLine({ row, onPrefill }: { row: FeedRow; onPrefill: (row: FeedRow) => void }) {
  const src =
    row.short !== null
      ? `dev ${row.short}${row.instance !== null ? ` / in ${row.instance}` : ''}`
      : row.scheme !== null
        ? `scheme ${row.scheme}`
        : 'no identity'
  const ev =
    row.event ??
    (row.lifecycle
      ? 'power / lifecycle'
      : row.info !== null
        ? `event 0x${row.info.toString(16).toUpperCase().padStart(3, '0')}`
        : 'input event')
  return (
    <div class="ln">
      <span class="ts">{row.at}</span>
      <span class="src">{src}</span>
      <span class="ev">{ev}</span>
      <span class="arrow">→</span>
      {row.short === null ? (
        <>
          <span class="rule">
            <span class="none">source unattributable</span>
          </span>
          <span class="mk amb">ambiguous</span>
        </>
      ) : row.rule === undefined && Date.now() - row.atMs < RULE_SETTLE_MS ? (
        <span class="rule">
          <span class="none">…</span>
        </span>
      ) : row.rule ? (
        <>
          <span class="rule">«{row.rule}»</span>
          <span class={`mk ${row.partial ? 'amb' : 'ok'}`}>
            {row.partial ? PARTIAL_REASONS[row.partial] ?? 'partial' : 'ok'}
          </span>
          {row.ms !== null && row.ms !== undefined && <span class="mk mk-ms">{row.ms} ms</span>}
        </>
      ) : (
        <>
          <span class="rule">
            <span class="none">no rule fired</span>
          </span>
          <button class="mkbtn" onClick={() => onPrefill(row)}>
            create a rule for this
          </button>
        </>
      )}
    </div>
  )
}
