import assert from 'node:assert/strict'
import test from 'node:test'

import {
  appendRow,
  attachActivation,
  FEED_CAPACITY,
  feedRow,
  nextSettleAt,
  RULE_SETTLE_MS,
  rowVerdict,
  type FeedRow,
} from '../src/rule-feed.js'

const stamp = (seq: number, atMs: number) => ({ seq, at: `t${seq}`, atMs })

const press = (seq: number, atMs: number, short: number | null = 3): FeedRow =>
  feedRow(
    { short_address: short, instance_number: 0, event: 'short_press', scheme: 2, event_info: 2 },
    false,
    stamp(seq, atMs),
  )

test('an input frame becomes a feed row, every missing field null', () => {
  assert.deepEqual(feedRow({}, true, stamp(1, 1000)), {
    seq: 1,
    at: 't1',
    short: null,
    instance: null,
    event: null,
    scheme: null,
    info: null,
    lifecycle: true,
    atMs: 1000,
  })
  assert.deepEqual(press(2, 2000), {
    seq: 2,
    at: 't2',
    short: 3,
    instance: 0,
    event: 'short_press',
    scheme: 2,
    info: 2,
    lifecycle: false,
    atMs: 2000,
  })
})

test('the feed keeps the newest rows up to its capacity', () => {
  let rows: FeedRow[] = []
  for (let seq = 1; seq <= FEED_CAPACITY + 5; seq += 1) rows = appendRow(rows, press(seq, seq))
  assert.equal(rows.length, FEED_CAPACITY)
  assert.equal(rows[0]?.seq, 6)
  assert.equal(rows.at(-1)?.seq, FEED_CAPACITY + 5)
})

test('an activation lands on the newest row still waiting for one, in arrival order', () => {
  const rows = [press(1, 1000), press(2, 1100)]
  const first = attachActivation(rows, { rule_name: 'коридор', partial: 0, trigger_to_publish_ms: 42 }, 1150)
  assert.equal(first[0]?.rule, undefined)
  assert.deepEqual(
    { rule: first[1]?.rule, partial: first[1]?.partial, ms: first[1]?.ms },
    { rule: 'коридор', partial: 0, ms: 42 },
  )
  const second = attachActivation(first, { rule_name: 'ночь' }, 1200)
  assert.deepEqual(
    { rule: second[0]?.rule, partial: second[0]?.partial, ms: second[0]?.ms },
    { rule: 'ночь', partial: 0, ms: null },
  )
  assert.equal(rows[1]?.rule, undefined)
})

test('a dry run leaves the feed untouched', () => {
  const rows = [press(1, 1000)]
  assert.equal(attachActivation(rows, { rule_name: 'x', dry: true }, 1100), rows)
})

test('a row past its settle window takes no activation', () => {
  const rows = [press(1, 1000)]
  assert.equal(attachActivation(rows, { rule_name: 'x' }, 1000 + RULE_SETTLE_MS), rows)
})

test('a row without a source is passed over for an older one with a source', () => {
  const rows = [press(1, 1000), press(2, 1100, null)]
  const next = attachActivation(rows, { rule_name: 'x' }, 1150)
  assert.equal(next[0]?.rule, 'x')
  assert.equal(next[1]?.rule, undefined)
})

test('a cut-short activation keeps its reason and duration', () => {
  const [row] = attachActivation(
    [press(1, 1000)],
    { rule_name: 'x', partial: 2, trigger_to_publish_ms: 7 },
    1001,
  )
  assert.equal(row?.partial, 2)
  assert.equal(row?.ms, 7)
})

test('a row is pending inside its window, then fired or unmatched; one without a source is unattributable', () => {
  const row = press(1, 1000)
  assert.equal(rowVerdict(row, 1000 + RULE_SETTLE_MS - 1), 'pending')
  assert.equal(rowVerdict(row, 1000 + RULE_SETTLE_MS), 'unmatched')
  assert.equal(rowVerdict({ ...row, rule: 'x' }, 1001), 'fired')
  assert.equal(rowVerdict({ ...row, rule: null }, 1001), 'unmatched')
  assert.equal(rowVerdict(press(2, 1000, null), 1001), 'unattributable')
})

test('the feed wakes at the soonest end of a pending window and never for a settled row', () => {
  const rows = [press(1, 1000), press(2, 1300), { ...press(3, 1400), rule: 'x' }, press(4, 1500, null)]
  assert.equal(nextSettleAt(rows, 1350), 1000 + RULE_SETTLE_MS)
  assert.equal(nextSettleAt(rows, 1000 + RULE_SETTLE_MS), 1300 + RULE_SETTLE_MS)
  assert.equal(nextSettleAt(rows, 1300 + RULE_SETTLE_MS), null)
})
