import { pad2 } from './format.js'
import { lexLine, NEWLINE, QUOTE } from './rule-lexis.js'

export type NameKind = 'lamp' | 'group' | 'input' | 'schedule'

export const MAX_NAME_BYTES = 48

export const SUGGEST_WIDTH_PX = 300
export const SUGGEST_MAX_HEIGHT_PX = 240

export const NAMES_FRESH_MS = 10_000

export interface NameCandidate {
  name: string
  id: number | null
}

export type RegistryNames = Record<NameKind, NameCandidate[]>

export interface RegistrySources {
  lamps: readonly { virtual_lamp_id: number; name: string }[]
  groups: readonly { group_id: number; name: string }[]
  inputs: readonly { short_address: number; name: string | null }[]
  schedules: readonly { schedule_id: string }[]
}

export interface CompletionContext {
  kind: NameKind
  openAt: number
  caret: number
  closeAt: number | null
  prefix: string
}

export type Unwritable = 'quote' | 'line_break' | 'too_long'

export interface Suggestion {
  name: string
  id: number | null
  text: string
  why: Unwritable | null
}

export interface Edit {
  from: number
  to: number
  insert: string
}

export interface KeyInput {
  key: string
  keyCode: number
  shiftKey: boolean
  ctrlKey: boolean
  altKey: boolean
  metaKey: boolean
  isComposing: boolean
}

export type KeyAction =
  | { kind: 'open' }
  | { kind: 'move'; active: number }
  | { kind: 'accept' }
  | { kind: 'close' }

export interface CompletionList {
  context: CompletionContext
  active: number
}

export interface KeyTarget {
  active: number
  count: number
}

export interface PlacementInput {
  x: number
  lineTop: number
  lineHeight: number
  viewWidth: number
  viewHeight: number
}

export interface Placement {
  left: number
  width: number
  top: number | null
  bottom: number | null
  maxHeight: number
}

const OPEN_PAREN = '('
const MEMBER_DOT = '.'
const HCL_OBJECT = 'hcl'
const OPEN_CHORD_KEY = ' '
const IME_PROCESS_KEY_CODE = 229

const CALLS: ReadonlyMap<string, NameKind> = new Map([
  ['lamp', 'lamp'],
  ['group', 'group'],
  ['input', 'input'],
])

const HCL_SCHEDULE_VERBS: ReadonlySet<string> = new Set(['enable', 'disable'])

const AFTER_FIRST_ARGUMENT: ReadonlySet<string> = new Set([')', ','])

const IDENT_CHAR = /[A-Za-z0-9_]/
const WHITESPACE = /\s/
const LINE_BREAK = /[\r\n]/

const UTF8 = new TextEncoder()

const isIdentChar = (ch: string) => IDENT_CHAR.test(ch)
const isBlank = (ch: string) => ch !== NEWLINE && WHITESPACE.test(ch)

function lineStartOf(text: string, index: number): number {
  return index === 0 ? 0 : text.lastIndexOf(NEWLINE, index - 1) + 1
}

function lineEndOf(text: string, index: number): number {
  const end = text.indexOf(NEWLINE, index)
  return end < 0 ? text.length : end
}

function lastNonBlankBefore(text: string, lineStart: number, index: number): number {
  let i = index - 1
  while (i >= lineStart && isBlank(text[i])) i -= 1
  return i
}

function wordEndingAt(
  text: string,
  lineStart: number,
  end: number,
): { word: string; start: number } {
  let start = end + 1
  while (start - 1 >= lineStart && isIdentChar(text[start - 1])) start -= 1
  return { word: text.slice(start, end + 1), start }
}

function charAt(text: string, lineStart: number, index: number): string | null {
  return index >= lineStart ? text[index] : null
}

function calleeBefore(text: string, lineStart: number, openAt: number): NameKind | null {
  const paren = lastNonBlankBefore(text, lineStart, openAt)
  if (charAt(text, lineStart, paren) !== OPEN_PAREN) return null
  const callee = wordEndingAt(text, lineStart, lastNonBlankBefore(text, lineStart, paren))
  const call = CALLS.get(callee.word)
  if (call !== undefined) return call
  if (!HCL_SCHEDULE_VERBS.has(callee.word)) return null
  const dot = lastNonBlankBefore(text, lineStart, callee.start)
  if (charAt(text, lineStart, dot) !== MEMBER_DOT) return null
  const object = wordEndingAt(text, lineStart, lastNonBlankBefore(text, lineStart, dot))
  return object.word === HCL_OBJECT ? 'schedule' : null
}

function closesArgument(line: string, close: number): boolean {
  let i = close + 1
  while (i < line.length && isBlank(line[i])) i += 1
  return i < line.length && AFTER_FIRST_ARGUMENT.has(line[i])
}

export function completionContext(text: string, caret: number): CompletionContext | null {
  if (caret < 0 || caret > text.length) return null
  const lineStart = lineStartOf(text, caret)
  const line = text.slice(lineStart, lineEndOf(text, caret))
  const at = caret - lineStart
  const literal = lexLine(line).strings.find(
    (s) => s.open < at && (s.close === null || at <= s.close),
  )
  if (literal === undefined) return null
  const openAt = lineStart + literal.open
  const kind = calleeBefore(text, lineStart, openAt)
  if (kind === null) return null
  const close = literal.close
  return {
    kind,
    openAt,
    caret,
    closeAt: close !== null && closesArgument(line, close) ? lineStart + close : null,
    prefix: text.slice(openAt + 1, caret),
  }
}

export function completionAt(
  text: string,
  selectionStart: number,
  selectionEnd: number,
): CompletionContext | null {
  return selectionStart === selectionEnd ? completionContext(text, selectionStart) : null
}

export function sameArgument(a: CompletionContext, b: CompletionContext): boolean {
  return a.kind === b.kind && a.openAt === b.openAt
}

export function listAfterInput(
  list: CompletionList | null,
  context: CompletionContext | null,
): { list: CompletionList | null; opening: boolean } {
  if (context === null) return { list: null, opening: false }
  return { list: { context, active: 0 }, opening: list === null }
}

export function listAfterCaret(
  list: CompletionList | null,
  context: CompletionContext | null,
): CompletionList | null {
  if (list === null || context === null || !sameArgument(context, list.context)) return null
  return context.caret === list.context.caret ? list : { context, active: list.active }
}

export function acceptable(list: CompletionList | null, context: CompletionContext | null): boolean {
  return list !== null && context !== null && sameArgument(context, list.context)
}

export function clampActive(active: number, count: number): number {
  return Math.min(active, Math.max(0, count - 1))
}

export function keyTarget(list: CompletionList | null, count: number, visible: boolean): KeyTarget | null {
  if (list === null) return null
  return { active: clampActive(list.active, count), count: visible ? count : 0 }
}

export function idTag(kind: NameKind, id: number | null): string | null {
  if (id === null) return null
  switch (kind) {
    case 'lamp':
      return `VL ${pad2(id)}`
    case 'group':
      return `G${id}`
    case 'input':
      return `SA ${pad2(id)}`
    case 'schedule':
      return null
  }
}

export function byIdHint(kind: NameKind, id: number): string {
  return kind === 'input' ? `input(${id}, …)` : `${kind}(${id})`
}

export function namesDue(requestedAt: number | null, now: number): boolean {
  return requestedAt === null || now - requestedAt >= NAMES_FRESH_MS
}

export function unwritable(name: string): Unwritable | null {
  if (name.includes(QUOTE)) return 'quote'
  if (LINE_BREAK.test(name)) return 'line_break'
  if (UTF8.encode(name).length > MAX_NAME_BYTES) return 'too_long'
  return null
}

function suggestionFor({ name, id }: NameCandidate): Suggestion | null {
  const why = unwritable(name)
  if (why === null) return { name, id, text: `${QUOTE}${name}${QUOTE}`, why }
  return id === null ? null : { name, id, text: String(id), why }
}

const fold = (s: string) => s.normalize('NFC').toLowerCase()

const compareText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0)

const PREFIX_TIER = 0
const SUBSTRING_TIER = 1

interface Ranked {
  tier: number
  key: string
  suggestion: Suggestion
}

function byTierThenName(a: Ranked, b: Ranked): number {
  return (
    a.tier - b.tier ||
    compareText(a.key, b.key) ||
    compareText(a.suggestion.name, b.suggestion.name)
  )
}

export function rankSuggestions(prefix: string, candidates: readonly NameCandidate[]): Suggestion[] {
  const query = fold(prefix)
  const seen = new Set<string>()
  const ranked: Ranked[] = []
  for (const candidate of candidates) {
    if (candidate.name === '' || seen.has(candidate.name)) continue
    seen.add(candidate.name)
    const key = fold(candidate.name)
    const at = key.indexOf(query)
    const suggestion = at < 0 ? null : suggestionFor(candidate)
    if (suggestion !== null) {
      ranked.push({ tier: at === 0 ? PREFIX_TIER : SUBSTRING_TIER, key, suggestion })
    }
  }
  return ranked.sort(byTierThenName).map((r) => r.suggestion)
}

export function acceptEdit(context: CompletionContext, suggestion: Suggestion): Edit {
  const to = context.closeAt === null ? context.caret : context.closeAt + 1
  return { from: context.openAt, to, insert: suggestion.text }
}

export function applyEdit(text: string, edit: Edit): { text: string; caret: number } {
  return {
    text: text.slice(0, edit.from) + edit.insert + text.slice(edit.to),
    caret: edit.from + edit.insert.length,
  }
}

export function keyAction(key: KeyInput, list: KeyTarget | null): KeyAction | null {
  if (key.isComposing || key.keyCode === IME_PROCESS_KEY_CODE) return null
  if (key.altKey || key.metaKey || key.shiftKey) return null
  if (key.ctrlKey) return key.key === OPEN_CHORD_KEY && list === null ? { kind: 'open' } : null
  if (list === null || list.count === 0) return null
  switch (key.key) {
    case 'ArrowDown':
      return { kind: 'move', active: (list.active + 1) % list.count }
    case 'ArrowUp':
      return { kind: 'move', active: (list.active - 1 + list.count) % list.count }
    case 'Enter':
    case 'Tab':
      return { kind: 'accept' }
    case 'Escape':
      return { kind: 'close' }
    default:
      return null
  }
}

export function lineAt(text: string, index: number): { line: number; start: number } {
  let line = 0
  for (let i = 0; i < index; i += 1) if (text[i] === NEWLINE) line += 1
  return { line, start: lineStartOf(text, index) }
}

export function placeList(at: PlacementInput): Placement | null {
  const lineBottom = at.lineTop + at.lineHeight
  if (lineBottom <= 0 || at.lineTop >= at.viewHeight) return null
  const width = Math.min(SUGGEST_WIDTH_PX, at.viewWidth)
  const left = Math.max(0, Math.min(at.x, at.viewWidth - width))
  const below = at.viewHeight - lineBottom
  const above = at.lineTop
  if (below >= SUGGEST_MAX_HEIGHT_PX || below >= above) {
    const maxHeight = Math.min(SUGGEST_MAX_HEIGHT_PX, below)
    return { left, width, top: lineBottom, bottom: null, maxHeight }
  }
  const maxHeight = Math.min(SUGGEST_MAX_HEIGHT_PX, above)
  return { left, width, top: null, bottom: at.viewHeight - at.lineTop, maxHeight }
}

export function scrollToShow(
  itemTop: number,
  itemHeight: number,
  scrollTop: number,
  viewHeight: number,
): number | null {
  if (itemTop < scrollTop) return itemTop
  const itemBottom = itemTop + itemHeight
  return itemBottom > scrollTop + viewHeight ? itemBottom - viewHeight : null
}

export function registryNames(sources: RegistrySources): RegistryNames {
  return {
    lamp: sources.lamps.map((l) => ({ name: l.name, id: l.virtual_lamp_id })),
    group: sources.groups.map((g) => ({ name: g.name, id: g.group_id })),
    input: sources.inputs.flatMap((d) => (d.name ? [{ name: d.name, id: d.short_address }] : [])),
    schedule: sources.schedules.map((s) => ({ name: s.schedule_id, id: null })),
  }
}
