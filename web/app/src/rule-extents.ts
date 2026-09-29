import { codeOf, NEWLINE } from './rule-lexis.js'

export interface RuleSpan {
  from: number
  to: number
}

const RULE_HEADER = /^\s*rule\s*"([^"]+)"/
const OPEN_BRACE = '{'
const CLOSE_BRACE = '}'

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
