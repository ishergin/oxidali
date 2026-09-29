import assert from 'node:assert/strict'
import test from 'node:test'

import {
  currentScope,
  editInScope,
  errorRow,
  insertBlock,
  insertionPoint,
  prependBlock,
  ruleSpans,
  ruleText,
  scopeRule,
  spliceRule,
} from '../src/rule-extents.js'

const doc = (...lines: string[]) => lines.join('\n')

const spansOf = (source: string) => Object.fromEntries(ruleSpans(source))

test('each rule spans its header to its closing brace, and a def block is not a rule', () => {
  const source = doc(
    '# ночной свет',
    'def "ночной контур" {',
    '  scene(13).recall(group("ночь"))',
    '}',
    'rule "ночь: вход" {',
    '  when input(dev=5, inst=0) becomes occupied',
    '  do   call("ночной контур")',
    '}',
    'rule "ушёл" { when every 5m do broadcast.off() }',
    '',
  )
  assert.deepEqual(spansOf(source), {
    'ночь: вход': { from: 4, to: 7 },
    'ушёл': { from: 8, to: 8 },
  })
})

test('a brace inside a string neither closes nor opens a block', () => {
  const source = doc(
    'rule "a" {',
    '  when every 5m',
    '  do   log("}")',
    '       log("still a")',
    '}',
    'rule "b" {',
    '  when every 5m',
    '  do   log("{")',
    '}',
    'rule "c" {',
    '  when every 5m',
    '  do   log("c")',
    '}',
  )
  assert.deepEqual(spansOf(source), {
    a: { from: 0, to: 4 },
    b: { from: 5, to: 8 },
    c: { from: 9, to: 12 },
  })
})

test('a brace inside a comment does not count', () => {
  const source = doc(
    'rule "a" {   # } closes nothing',
    '  when every 5m',
    '  do   log("a")   # {',
    '}',
    'rule "b" {',
    '  when every 5m',
    '  do   log("b")',
    '}',
  )
  assert.deepEqual(spansOf(source), { a: { from: 0, to: 3 }, b: { from: 4, to: 7 } })
})

test('a # inside a string is not a comment, so the brace after it still closes the rule', () => {
  assert.deepEqual(spansOf('rule "решётка" { when every 5m do log("#1") }'), {
    'решётка': { from: 0, to: 0 },
  })
})

test('an unterminated string hides the braces after its quote', () => {
  const source = doc(
    'rule "a" {',
    '  when every 5m',
    '  do   log("oops {',
    '}',
    'rule "b" {',
    '  when every 5m',
    '  do   log("b")',
    '}',
  )
  assert.deepEqual(spansOf(source), { a: { from: 0, to: 3 }, b: { from: 4, to: 7 } })
})

test('modifiers may stand between the rule name and its brace', () => {
  const source = doc(
    'rule "night" enabled false cooldown 500ms {',
    '  when at 23:00',
    '  do   broadcast.off()',
    '}',
  )
  assert.deepEqual(spansOf(source), { night: { from: 0, to: 3 } })
})

test('a header whose only brace is quoted or commented opens nothing', () => {
  const source = doc(
    'rule "x" # {',
    'rule "y" {',
    '  when every 5m',
    '  do   log("y")',
    '}',
  )
  assert.deepEqual(spansOf(source), { y: { from: 1, to: 4 } })
})

test('nested blocks keep the rule open until its own closing brace', () => {
  const source = doc(
    'rule "ушёл" {',
    '  when input(dev=3, inst=2) is long_press_start',
    '  do   broadcast.off()',
    '       if time in 23:00 .. sunrise { log("ночь") } else { log("день") }',
    '       after 5m do {',
    '         hcl.resume(broadcast)',
    '       }',
    '}',
    'rule "b" { when every 5m do log("b") }',
  )
  assert.deepEqual(spansOf(source), { 'ушёл': { from: 0, to: 7 }, b: { from: 8, to: 8 } })
})

test('the single-rule view splices back without touching any other byte', () => {
  const source = doc(
    '# шапка',
    'rule "a" {',
    '  when every 5m',
    '  do   log("}")',
    '}',
    '',
    'rule "b" {',
    '  when every 5m',
    '  do   log("b")',
    '}',
    '',
  )
  const spans = ruleSpans(source)
  for (const span of spans.values()) {
    assert.equal(spliceRule(source, span, ruleText(source, span)), source)
  }
  const a = spans.get('a')
  assert.ok(a)
  assert.equal(ruleText(source, a), doc('rule "a" {', '  when every 5m', '  do   log("}")', '}'))
  const edited = spliceRule(source, a, doc('rule "a" {', '  when every 1m', '  do   log("}")', '}'))
  assert.equal(edited, source.replace('every 5m\n  do   log("}")', 'every 1m\n  do   log("}")'))
})

const away = doc(
  'rule "ушёл" {',
  '  when input(dev=3, inst=2) is long_press_start',
  '  do   broadcast.off()',
  '       after 5m do { hcl.resume(broadcast) }',
  '}',
  'rule "b" {',
  '  when every 5m',
  '  do   log("b")',
  '}',
  '',
)

const ruleB = doc('rule "b" {', '  when every 5m', '  do   log("b")', '}', '')

test("a rule's own tab keeps its rule while an edit leaves a string open", () => {
  const scope = scopeRule('ушёл', away)
  assert.ok(scope)
  const typed = ruleText(away, scope.span).replace('hcl.resume(broadcast)', 'hcl.resume(lamp(")')
  const next = editInScope(scope, typed)
  assert.equal(ruleSpans(next.base).has('ушёл'), false)
  assert.equal(currentScope(next, next.base), next)
  assert.equal(ruleText(next.base, next.span), typed)
  assert.ok(next.base.endsWith(ruleB))
})

test('the tab follows its rule when an edit adds or removes lines', () => {
  const scope = scopeRule('ушёл', away)
  assert.ok(scope)
  const longer = editInScope(scope, doc(ruleText(away, scope.span), '# ещё строка'))
  assert.deepEqual(longer.span, { from: 0, to: 5 })
  assert.ok(longer.base.endsWith(ruleB))
  const shorter = editInScope(longer, 'rule "ушёл" { when every 5m do broadcast.off() }')
  assert.deepEqual(shorter.span, { from: 0, to: 0 })
  assert.equal(shorter.base, doc('rule "ушёл" { when every 5m do broadcast.off() }', ruleB))
})

test('a document changed outside the tab finds its rule again by name, or lets it go', () => {
  const scope = scopeRule('ушёл', away)
  assert.ok(scope)
  const moved = doc('# шапка', away)
  assert.deepEqual(currentScope(scope, moved), { name: 'ушёл', span: { from: 1, to: 5 }, base: moved })
  assert.equal(currentScope(scope, ruleB), null)
  assert.equal(currentScope(null, away), null)
})

test("a snippet from a rule's own tab lands after that rule and the document stays whole", () => {
  const scope = scopeRule('ушёл', away)
  assert.ok(scope)
  const snippet = doc('rule "c" {', '  when every 1m', '  do   log("c")', '}')
  const caretInsideTheRule = 3
  const next = insertBlock(away, insertionPoint(away, scope, caretInsideTheRule), snippet)
  assert.equal(next, doc(ruleText(away, scope.span), '', snippet, '', ruleB))
  assert.equal(next.split('rule "ушёл"').length, 2)
})

test('a block goes in with one blank line around it, at the caret or at the end', () => {
  assert.equal(insertionPoint('abc', null, 1), 1)
  assert.equal(insertionPoint('abc', null, null), 3)
  assert.equal(insertBlock('', 0, 'X'), 'X\n')
  assert.equal(insertBlock('a\n', 2, '  X\n'), 'a\n\nX\n')
  assert.equal(insertBlock('a\n\nb\n', 3, 'X'), 'a\n\nX\n\nb\n')
  assert.equal(insertBlock('a\nb', 1, 'X'), 'a\n\nX\n\nb')
  assert.equal(insertBlock('a\n\nb', 1, 'X'), 'a\n\nX\n\nb')
})

test('a feed skeleton goes on top of the document', () => {
  assert.equal(prependBlock('', 'X\n'), 'X\n')
  assert.equal(prependBlock('a\n', 'X\n'), 'X\n\na\n')
})

test("a parse error's row counts from the top of what the editor shows, and a line outside a rule's tab has none", () => {
  const scope = scopeRule('ушёл', doc('# шапка', '', away))
  assert.ok(scope)
  assert.deepEqual(scope.span, { from: 2, to: 6 })
  assert.equal(errorRow(5, null), 4)
  assert.equal(errorRow(3, scope), 0)
  assert.equal(errorRow(5, scope), 2)
  assert.equal(errorRow(7, scope), 4)
  assert.equal(errorRow(2, scope), null)
  assert.equal(errorRow(8, scope), null)
})
