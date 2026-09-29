import { useEffect, useRef, useState } from 'preact/hooks'

import { api, ApiError, isRulesParseError } from '../api/client'
import type {
  RuleJson,
  RuleRuntime,
  RulesDocument,
  RulesDocumentJson,
  RulesParseResult,
} from '../api/types'
import { connection, subscribe, type WsChannel, type WsEvent } from '../api/ws'
import { ADAPTER, deviceClock, timestamp, UNANCHORED_CLOCK_HINT } from '../format'
import { freshSource, settledValue } from '../fresh-source'
import { usePoll } from '../hooks'
import {
  acceptable,
  acceptEdit,
  applyEdit,
  byIdHint,
  clampActive,
  completionAt,
  idTag,
  keyAction,
  keyTarget,
  lineAt,
  listAfterCaret,
  listAfterInput,
  MAX_NAME_BYTES,
  NAMES_FRESH_MS,
  placeList,
  rankSuggestions,
  registryNames,
  scrollToShow,
  type CompletionContext,
  type CompletionList,
  type Edit,
  type KeyInput,
  type NameKind,
  type Placement,
  type RegistryNames,
  type Suggestion,
  type ByIdReason,
} from '../rule-completion'
import {
  currentScope,
  editInScope,
  insertBlock,
  insertionPoint,
  prependBlock,
  ruleText,
  scopeRule,
  type RuleScope,
} from '../rule-extents'
import {
  appendRow,
  attachActivation,
  feedRow,
  feedSkeleton,
  nextSettleAt,
  rowVerdict,
  type FeedRow,
  type InputFeedPayload,
  type RuleActivationPayload,
} from '../rule-feed'
import { errorMessage, mutate, notify, opCommitted, trackOp } from '../toast'

const MAX_RULES_SOURCE_BYTES = 12_240

const PARSE_DEBOUNCE_MS = 500

const LINE_HEIGHT_PX = 20
const EDITOR_PAD_PX = 12
const EDITOR_PAD_X_PX = 14

const NAMES_RECONCILE_MS = 60_000

const SUGGEST_ID = 'rule-suggest'

const KIND_LABEL: Record<NameKind, string> = {
  lamp: 'Virtual lamps',
  group: 'Groups',
  input: 'Input devices',
  schedule: 'HCL schedules',
}

const BY_ID_WHY: Record<ByIdReason, string> = {
  quote: 'The name has a quote, which a rule string cannot hold',
  line_break: 'The name has a line break, which a rule string cannot hold',
  too_long: `The name is longer than the ${MAX_NAME_BYTES} bytes a rule string holds`,
  ambiguous: 'Several entries share this name, and the device would resolve it to any one of them',
}

const FEED_CHANNELS: WsChannel[] = ['input', 'rules']

const INPUT_EVENT = 'DaliInputEventObservedEvent'
const LIFECYCLE_EVENT = 'DaliInputDeviceLifecycleEvent'
const ACTIVATION_EVENT = 'RulesActivationEvent'

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

const PARTIAL_REASONS: Record<number, string> = {
  1: 'partial — condition',
  2: 'partial — effect budget',
  3: 'partial — chain depth',
}

async function fetchRegistryNames(previous: RegistryNames | null): Promise<RegistryNames> {
  const [lamps, groups, inputs, schedules] = await Promise.allSettled([
    api.virtualLamps(ADAPTER),
    api.groups(ADAPTER),
    api.inputDevices(ADAPTER),
    api.hclSchedules(),
  ])
  return registryNames(
    {
      lamps: settledValue(lamps)?.virtual_lamps ?? null,
      groups: settledValue(groups)?.groups ?? null,
      inputs: settledValue(inputs)?.input_devices ?? null,
      schedules: settledValue(schedules)?.schedules ?? null,
    },
    previous,
  )
}

function useRegistryNames() {
  const [source] = useState(() => freshSource(fetchRegistryNames, Date.now, NAMES_FRESH_MS))
  const poll = usePoll(source, NAMES_RECONCILE_MS)
  return { data: poll.data, refresh: () => void poll.reload() }
}

let measureCtx: CanvasRenderingContext2D | null = null

function textWidth(ta: HTMLTextAreaElement, text: string): number {
  measureCtx ??= document.createElement('canvas').getContext('2d')
  if (measureCtx === null) return 0
  const style = getComputedStyle(ta)
  measureCtx.font = `${style.fontStyle} ${style.fontWeight} ${style.fontSize} ${style.fontFamily}`
  return measureCtx.measureText(text).width
}

function suggestPlacement(
  ta: HTMLTextAreaElement,
  text: string,
  openAt: number,
  scroll: { top: number; left: number },
): Placement | null {
  const { line, start } = lineAt(text, openAt)
  return placeList({
    x: EDITOR_PAD_X_PX + textWidth(ta, text.slice(start, openAt + 1)) - scroll.left,
    lineTop: EDITOR_PAD_PX + line * LINE_HEIGHT_PX - scroll.top,
    lineHeight: LINE_HEIGHT_PX,
    viewWidth: ta.clientWidth,
    viewHeight: ta.clientHeight,
  })
}

function caretContext(ta: HTMLTextAreaElement | null): CompletionContext | null {
  return ta === null ? null : completionAt(ta.value, ta.selectionStart, ta.selectionEnd)
}

function insertKeepingUndo(ta: HTMLTextAreaElement, edit: Edit): string | null {
  ta.setSelectionRange(edit.from, edit.to)
  if (document.execCommand('insertText', false, edit.insert)) return null
  const next = applyEdit(ta.value, edit)
  ta.value = next.text
  ta.setSelectionRange(next.caret, next.caret)
  return next.text
}

function useNameCompletion(
  taRef: { current: HTMLTextAreaElement | null },
  names: RegistryNames | null,
  onOpening: () => void,
) {
  const [list, setList] = useState<CompletionList | null>(null)
  const items =
    list !== null && names !== null
      ? rankSuggestions(list.context.prefix, names[list.context.kind])
      : []
  const active = list === null ? 0 : clampActive(list.active, items.length)

  const open = () => {
    const next = listAfterInput(list, caretContext(taRef.current))
    if (next.opening) onOpening()
    setList(next.list)
  }

  const follow = () => setList(listAfterCaret(list, caretContext(taRef.current)))

  const accept = (suggestion: Suggestion | undefined): string | null => {
    const ta = taRef.current
    const context = caretContext(ta)
    const ok = acceptable(list, context)
    setList(null)
    if (ta === null || context === null || !ok || suggestion === undefined) return null
    return insertKeepingUndo(ta, acceptEdit(context, suggestion))
  }

  const keyDown = (e: KeyInput & { preventDefault: () => void }, visible: boolean): string | null => {
    const action = keyAction(e, keyTarget(list, items.length, visible))
    if (action === null) return null
    e.preventDefault()
    switch (action.kind) {
      case 'open':
        open()
        return null
      case 'move':
        setList(list && { ...list, active: action.active })
        return null
      case 'close':
        setList(null)
        return null
      case 'accept':
        return accept(items[active])
    }
  }

  return { list, items, active, open, follow, accept, keyDown, close: () => setList(null) }
}

function SuggestRow({
  kind,
  suggestion,
  index,
  active,
  onPick,
}: {
  kind: NameKind
  suggestion: Suggestion
  index: number
  active: boolean
  onPick: (s: Suggestion) => void
}) {
  const { name, id, why } = suggestion
  const tag = idTag(kind, id)
  return (
    <div
      id={`${SUGGEST_ID}-${index}`}
      role="option"
      aria-selected={active}
      class={`so${active ? ' on' : ''}${why !== null ? ' byid' : ''}`}
      title={why !== null ? BY_ID_WHY[why] : undefined}
      onMouseDown={(e) => {
        e.preventDefault()
        onPick(suggestion)
      }}
    >
      <span class="nm">{name}</span>
      {why !== null && id !== null ? (
        <span class="why">by id → {byIdHint(kind, id)}</span>
      ) : (
        tag !== null && <span class="tag">{tag}</span>
      )}
    </div>
  )
}

function SuggestList({
  kind,
  items,
  active,
  at,
  onPick,
}: {
  kind: NameKind
  items: Suggestion[]
  active: number
  at: Placement
  onPick: (s: Suggestion) => void
}) {
  const bodyRef = useRef<HTMLDivElement | null>(null)
  useEffect(() => {
    const body = bodyRef.current
    const row = body?.children[active]
    if (!body || !(row instanceof HTMLElement)) return
    const next = scrollToShow(row.offsetTop, row.offsetHeight, body.scrollTop, body.clientHeight)
    if (next !== null) body.scrollTop = next
  }, [active])
  const vertical = at.top !== null ? { top: `${at.top}px` } : { bottom: `${at.bottom}px` }
  const box = { left: `${at.left}px`, width: `${at.width}px`, maxHeight: `${at.maxHeight}px` }
  return (
    <div
      class="suggest"
      id={SUGGEST_ID}
      role="listbox"
      aria-label={KIND_LABEL[kind]}
      style={{ ...box, ...vertical }}
      onMouseDown={(e) => e.preventDefault()}
    >
      <div class="sh">
        <span>{KIND_LABEL[kind]}</span>
        <span>{items.length}</span>
      </div>
      <div class="sb" ref={bodyRef}>
        {items.map((s, i) => (
          <SuggestRow
            key={s.key}
            kind={kind}
            suggestion={s}
            index={i}
            active={i === active}
            onPick={onPick}
          />
        ))}
      </div>
      <div class="sf">
        <kbd>↑</kbd> <kbd>↓</kbd> move · <kbd>Enter</kbd> <kbd>Tab</kbd> insert · <kbd>Esc</kbd> close
      </div>
    </div>
  )
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
  const [scrollLeft, setScrollLeft] = useState(0)
  const [scope, setScope] = useState<RuleScope | null>(null)
  const parseSeq = useRef(0)
  const taRef = useRef<HTMLTextAreaElement | null>(null)
  const caretTouched = useRef(false)
  const names = useRegistryNames()
  const completion = useNameCompletion(taRef, names.data, names.refresh)

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
  const current = currentScope(scope, full)
  const shown = current === null ? full : ruleText(full, current.span)
  const dirty = draft !== null && draft !== source
  const drift = draft !== null && baseRev !== null && text.revision > baseRev
  const bytes = UTF8.encode(full).length
  const suggestAt =
    completion.list !== null && completion.items.length > 0 && taRef.current !== null
      ? suggestPlacement(taRef.current, shown, completion.list.context.openAt, {
          top: scrollTop,
          left: scrollLeft,
        })
      : null

  const editDraft = (value: string) => {
    if (draft === null) setBaseRev(data.text.revision)
    if (current === null) {
      setDraft(value)
      return
    }
    const next = editInScope(current, value)
    setScope(next)
    setDraft(next.base)
  }

  const editWhole = (next: string) => {
    if (draft === null) setBaseRev(data.text.revision)
    setScope(null)
    setDraft(next)
  }

  const pickName = (s: Suggestion) => {
    const next = completion.accept(s)
    if (next !== null) editDraft(next)
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
    const ta = taRef.current
    const caret = caretTouched.current && ta ? ta.selectionStart : null
    editWhole(insertBlock(full, insertionPoint(full, current, caret), code))
  }

  const prefill = (row: FeedRow) => {
    const skeleton = feedSkeleton(row)
    if (skeleton === null) return
    editWhole(prependBlock(full, skeleton))
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
              class={current === null ? 't on' : 't'}
              onClick={() => setScope(null)}
            >
              Whole document
            </span>
            {current !== null && <span class="t on">{current.name}</span>}
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
              aria-autocomplete="list"
              aria-controls={suggestAt ? SUGGEST_ID : undefined}
              aria-activedescendant={suggestAt ? `${SUGGEST_ID}-${completion.active}` : undefined}
              onFocus={() => {
                caretTouched.current = true
                names.refresh()
              }}
              onBlur={completion.close}
              onInput={(e) => {
                editDraft(e.currentTarget.value)
                completion.open()
              }}
              onKeyDown={(e) => {
                const next = completion.keyDown(e, suggestAt !== null)
                if (next !== null) editDraft(next)
              }}
              onKeyUp={completion.follow}
              onClick={completion.follow}
              onScroll={(e) => {
                setScrollTop(e.currentTarget.scrollTop)
                setScrollLeft(e.currentTarget.scrollLeft)
              }}
            />
            {suggestAt && completion.list && (
              <SuggestList
                kind={completion.list.context.kind}
                items={completion.items}
                active={completion.active}
                at={suggestAt}
                onPick={pickName}
              />
            )}
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
            selected={current?.name ?? null}
            onSelect={(name) => setScope(scopeRule(name, full))}
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
            <span class="rbody">
              <span class={r.enabled ? 'n' : 'n dis'} title={`Edit "${r.name}"`}>
                {r.name}
              </span>
              <span class="t">{ruleSummary(r)}</span>
              <RuntimeLine runtime={r.runtime} />
            </span>
            {r.runtime.last_outcome !== null && (
              <span class={`out ${r.runtime.last_outcome}`}>{r.runtime.last_outcome}</span>
            )}
          </div>
        ))
      )}
    </div>
  )
}

function RuntimeLine({ runtime }: { runtime: RuleRuntime }) {
  if (runtime.last_fired_at_ms === null) return <span class="meta">not fired since boot</span>
  const at = deviceClock(runtime.last_fired_at_ms)
  const latency = runtime.last_latency_ms === null ? '—' : `${runtime.last_latency_ms} ms`
  return (
    <span class="meta">
      {runtime.fire_count}× · {latency} ·{' '}
      <span title={at.anchored ? undefined : UNANCHORED_CLOCK_HINT}>{at.text}</span>
      {runtime.last_error !== null && (
        <>
          {' · '}
          <span class="why">{runtime.last_error}</span>
        </>
      )}
    </span>
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
      if (event.type === ACTIVATION_EVENT) {
        const activation = (event.payload ?? {}) as RuleActivationPayload
        setRows((prev) => attachActivation(prev, activation, Date.now()))
        return
      }
      if (event.type !== INPUT_EVENT && event.type !== LIFECYCLE_EVENT) return
      seq.current += 1
      const stamp = { seq: seq.current, at: timestamp(), atMs: Date.now() }
      const row = feedRow(
        (event.payload ?? {}) as InputFeedPayload,
        event.type === LIFECYCLE_EVENT,
        stamp,
      )
      setRows((prev) => appendRow(prev, row))
    })
    return dispose
  }, [])

  const [, setTick] = useState(0)
  const dueAt = nextSettleAt(rows, Date.now())
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
      <FeedOutcome row={row} onPrefill={onPrefill} />
    </div>
  )
}

function FeedOutcome({ row, onPrefill }: { row: FeedRow; onPrefill: (row: FeedRow) => void }) {
  switch (rowVerdict(row, Date.now())) {
    case 'unattributable':
      return (
        <>
          <span class="rule">
            <span class="none">source unattributable</span>
          </span>
          <span class="mk amb">ambiguous</span>
        </>
      )
    case 'pending':
      return (
        <span class="rule">
          <span class="none">…</span>
        </span>
      )
    case 'fired':
      return (
        <>
          <span class="rule">«{row.rule}»</span>
          <span class={`mk ${row.partial ? 'amb' : 'ok'}`}>
            {row.partial ? PARTIAL_REASONS[row.partial] ?? 'partial' : 'ok'}
          </span>
          {row.ms !== null && row.ms !== undefined && <span class="mk mk-ms">{row.ms} ms</span>}
        </>
      )
    case 'unmatched':
      return (
        <>
          <span class="rule">
            <span class="none">no rule fired</span>
          </span>
          <button class="mkbtn" onClick={() => onPrefill(row)}>
            create a rule for this
          </button>
        </>
      )
  }
}
