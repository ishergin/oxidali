import assert from 'node:assert/strict'
import test from 'node:test'

import { ruleSpans, ruleText, spliceRule } from '../src/rule-extents.js'

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
