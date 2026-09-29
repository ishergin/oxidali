import { codeOf, NEWLINE } from './rule-lexis.js'

export interface RuleSpan {
  from: number
  to: number
}

export interface RuleScope {
  name: string
  span: RuleSpan
  base: string
}

const RULE_HEADER = /^\s*rule\s*"([^"]+)"/
const OPEN_BRACE = '{'
const CLOSE_BRACE = '}'
const BLANK_LINE = NEWLINE + NEWLINE

function braceBalance(code: string): number {
  let balance = 0
  for (const ch of code) {
    if (ch === OPEN_BRACE) balance += 1
    else if (ch === CLOSE_BRACE) balance -= 1
  }
  return balance
}

export function ruleSpans(source: string): Map<string, RuleSpan> {
  const spans = new Map<string, RuleSpan>()
  const lines = source.split(NEWLINE)
  let open: { name: string; from: number; depth: number } | null = null
  for (let i = 0; i < lines.length; i += 1) {
    const code = codeOf(lines[i])
    if (open === null) {
      const header = RULE_HEADER.exec(lines[i])
      if (header === null || !code.includes(OPEN_BRACE)) continue
      open = { name: header[1], from: i, depth: 0 }
    }
    open.depth += braceBalance(code)
    if (open.depth <= 0) {
      spans.set(open.name, { from: open.from, to: i })
      open = null
    }
  }
  return spans
}

export function ruleText(source: string, span: RuleSpan): string {
  return source.split(NEWLINE).slice(span.from, span.to + 1).join(NEWLINE)
}

export function spliceRule(source: string, span: RuleSpan, text: string): string {
  const lines = source.split(NEWLINE)
  return [...lines.slice(0, span.from), ...text.split(NEWLINE), ...lines.slice(span.to + 1)].join(
    NEWLINE,
  )
}

export function scopeRule(name: string, source: string): RuleScope | null {
  const span = ruleSpans(source).get(name)
  return span === undefined ? null : { name, span, base: source }
}

export function currentScope(scope: RuleScope | null, source: string): RuleScope | null {
  return scope === null || scope.base === source ? scope : scopeRule(scope.name, source)
}

export function editInScope(scope: RuleScope, text: string): RuleScope {
  const lines = text.split(NEWLINE).length
  return {
    name: scope.name,
    span: { from: scope.span.from, to: scope.span.from + lines - 1 },
    base: spliceRule(scope.base, scope.span, text),
  }
}

function lineEnd(source: string, line: number): number {
  let end = -1
  for (let i = 0; i <= line; i += 1) {
    end = source.indexOf(NEWLINE, end + 1)
    if (end < 0) return source.length
  }
  return end
}

export function insertionPoint(source: string, scope: RuleScope | null, caret: number | null): number {
  if (scope !== null) return lineEnd(source, scope.span.to)
  return caret ?? source.length
}

export function insertBlock(source: string, at: number, block: string): string {
  const before = source.slice(0, at)
  const after = source.slice(at)
  const lead =
    before === '' || before.endsWith(BLANK_LINE) ? '' : before.endsWith(NEWLINE) ? NEWLINE : BLANK_LINE
  const tail =
    after === '' ? NEWLINE : after.startsWith(BLANK_LINE) ? '' : after.startsWith(NEWLINE) ? NEWLINE : BLANK_LINE
  return before + lead + block.trim() + tail + after
}

export function prependBlock(source: string, block: string): string {
  return source === '' ? block : block + NEWLINE + source
}
