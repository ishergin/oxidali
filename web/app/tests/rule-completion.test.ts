import assert from 'node:assert/strict'
import test from 'node:test'

import {
  acceptEdit,
  applyEdit,
  completionContext,
  keyAction,
  lineAt,
  MAX_NAME_BYTES,
  placeList,
  rankSuggestions,
  registryNames,
  scrollToShow,
  SUGGEST_MAX_HEIGHT_PX,
  SUGGEST_WIDTH_PX,
  unwritable,
  type KeyInput,
  type NameCandidate,
  type Suggestion,
} from '../src/rule-completion.js'

const CARET = '▮'

function at(marked: string): { text: string; caret: number } {
  const caret = marked.indexOf(CARET)
  assert.ok(caret >= 0, `no caret in ${marked}`)
  return { text: marked.slice(0, caret) + marked.slice(caret + CARET.length), caret }
}

function contextOf(marked: string) {
  const { text, caret } = at(marked)
  return completionContext(text, caret)
}

function only(prefix: string, name: string, id: number | null = 1): Suggestion {
  const [suggestion] = rankSuggestions(prefix, [{ name, id }])
  assert.ok(suggestion, `${name} is not suggested for "${prefix}"`)
  return suggestion
}

function accept(marked: string, suggestion: Suggestion): string {
  const { text, caret } = at(marked)
  const context = completionContext(text, caret)
  assert.ok(context, `no completion place in ${marked}`)
  const next = applyEdit(text, acceptEdit(context, suggestion))
  return next.text.slice(0, next.caret) + CARET + next.text.slice(next.caret)
}

const names = (list: Suggestion[]) => list.map((s) => s.name)

const candidates = (...list: string[]): NameCandidate[] => list.map((name, id) => ({ name, id }))

const key = (k: string, mods: Partial<KeyInput> = {}): KeyInput => ({
  key: k,
  shiftKey: false,
  ctrlKey: false,
  altKey: false,
  metaKey: false,
  isComposing: false,
  ...mods,
})

test('the first string argument of lamp, group and input is a completion place', () => {
  assert.deepEqual(contextOf('do   lamp("ку▮'), {
    kind: 'lamp',
    openAt: 10,
    caret: 13,
    closeAt: null,
    prefix: 'ку',
  })
  const group = contextOf('when group("кухня ост▮") becomes any_on')
  assert.equal(group?.kind, 'group')
  assert.equal(group?.prefix, 'кухня ост')
  assert.equal(contextOf('when input("пан▮", inst=1) is short_press')?.kind, 'input')
  assert.equal(contextOf('if hcl is enabled for group("▮")')?.prefix, '')
  assert.equal(contextOf('do scene(13).recall(group("ноч▮')?.kind, 'group')
  assert.equal(contextOf('do lamp ( "ку▮')?.kind, 'lamp')
})

test('hcl.enable and hcl.disable complete schedule ids, blanks between tokens included', () => {
  assert.equal(contextOf('do hcl.enable("mor▮')?.kind, 'schedule')
  assert.equal(contextOf('do hcl . disable ( "▮")')?.kind, 'schedule')
  assert.equal(contextOf('do hcl.resume("▮'), null)
  assert.equal(contextOf('do rule("ночь").enable("▮'), null)
  assert.equal(contextOf('do enable("▮'), null)
})

test('the strings of other calls and of a rule header are not completion places', () => {
  for (const marked of [
    'rule "ку▮',
    'def "ноч▮',
    'do var("ре▮',
    'do var("режим").set("н▮',
    'do timer("ночь▮',
    'do call("ноч▮',
    'when rule("X▮") fails',
    'do log("▮',
    'do mqtt.publish("▮',
    'when device("▮',
  ]) {
    assert.equal(contextOf(marked), null, marked)
  }
})

test('the callee is a whole word, looked up by name only', () => {
  assert.equal(contextOf('do mylamp("▮'), null)
  assert.equal(contextOf('do lamp_2("▮'), null)
  assert.equal(contextOf('do constructor("▮'), null)
})

test('only a string right after the opening parenthesis completes', () => {
  assert.equal(contextOf('do lamp(3, "▮'), null)
  assert.equal(contextOf('when input(dev="▮'), null)
  assert.equal(contextOf('when input device("▮'), null)
})

test('the caret has to be inside the literal', () => {
  assert.equal(contextOf('do lamp(▮"кухня")'), null)
  assert.equal(contextOf('do lamp("кухня"▮)'), null)
  assert.equal(contextOf('do lamp("кухня")▮'), null)
  assert.equal(contextOf('do lamp("▮кухня")')?.prefix, '')
})

test('quote parity decides where a literal is, and # inside one is not a comment', () => {
  assert.equal(contextOf('do log("#1") lamp("ку▮')?.prefix, 'ку')
  assert.equal(contextOf('do lamp("#1 ку▮')?.prefix, '#1 ку')
  assert.equal(contextOf('do var("x").set("lamp("▮)'), null)
  assert.equal(contextOf('# do lamp("ку▮'), null)
  assert.equal(contextOf('do lamp("к") # lamp("▮'), null)
})

test('a string cannot span lines, so every line starts outside one', () => {
  assert.equal(contextOf('do log("oops\ndo lamp("ку▮')?.prefix, 'ку')
  assert.equal(contextOf('do lamp(\n  "ку▮'), null)
})

test('the literal ends at the next quote on its line, where the device lexer ends it', () => {
  const { text, caret } = at('do lamp("кух▮ня").on()')
  const context = completionContext(text, caret)
  assert.ok(context?.closeAt)
  assert.equal(text.slice(context.openAt + 1, context.closeAt), 'кухня')
  assert.equal(contextOf('do lamp("ку▮\ndo log("x")')?.closeAt, null)
})

test('a caret outside the text has no context', () => {
  assert.equal(completionContext('lamp("', -1), null)
  assert.equal(completionContext('lamp("', 7), null)
  assert.equal(completionContext('', 0), null)
})

test('prefix matches come first, then substring matches, both case-insensitive', () => {
  const list = candidates('окно кухни', 'Кухня-2', 'прихожая', 'кухня')
  assert.deepEqual(names(rankSuggestions('КУХ', list)), ['кухня', 'Кухня-2', 'окно кухни'])
  const rooms = candidates('кухня остров', 'Остров', 'гостиная')
  assert.deepEqual(names(rankSuggestions('ост', rooms)), ['Остров', 'гостиная', 'кухня остров'])
})

test('an empty prefix lists every name alphabetically, spaces and all', () => {
  const list = candidates('прихожая', 'кухня остров', 'кухня', 'Hall light')
  assert.deepEqual(names(rankSuggestions('', list)), [
    'Hall light',
    'кухня',
    'кухня остров',
    'прихожая',
  ])
})

test('a name matches in either Unicode normalisation and is inserted as stored', () => {
  const stored = 'Спальня й'.normalize('NFD')
  const [match] = rankSuggestions('спальня й'.normalize('NFC'), [{ name: stored, id: 2 }])
  assert.equal(match?.name, stored)
  assert.equal(match?.text, `"${stored}"`)
})

test('a name listed twice is suggested once, an empty one never', () => {
  const list: NameCandidate[] = [
    { name: 'кухня', id: 1 },
    { name: 'кухня', id: 2 },
    { name: '', id: 3 },
  ]
  assert.deepEqual(rankSuggestions('', list), [
    { name: 'кухня', id: 1, text: '"кухня"', why: null },
  ])
})

test('nothing matches, nothing is suggested', () => {
  assert.deepEqual(rankSuggestions('гараж', candidates('кухня', 'прихожая')), [])
})

test('the name limit counts UTF-8 bytes, not characters', () => {
  const cyrillicLetters = MAX_NAME_BYTES / 2
  assert.equal(unwritable('ж'.repeat(cyrillicLetters)), null)
  assert.equal(unwritable('ж'.repeat(cyrillicLetters + 1)), 'too_long')
  assert.equal(unwritable('a'.repeat(MAX_NAME_BYTES)), null)
  assert.equal(unwritable('a'.repeat(MAX_NAME_BYTES + 1)), 'too_long')
})

test('a name a rule string cannot hold is offered by its id', () => {
  assert.deepEqual(only('', 'Бра "у зеркала"', 4), {
    name: 'Бра "у зеркала"',
    id: 4,
    text: '4',
    why: 'quote',
  })
  assert.equal(only('', 'две\nстроки', 5).why, 'line_break')
  assert.equal(only('', 'две\rстроки', 5).why, 'line_break')
  const long = 'кухня: подсветка рабочей зоны'
  assert.deepEqual(only('кух', long, 12), { name: long, id: 12, text: '12', why: 'too_long' })
})

test('a name with neither a string form nor an id is not offered', () => {
  assert.deepEqual(rankSuggestions('', [{ name: 'a"b', id: null }]), [])
})

test('accepting completes the name and adds the missing closing quote', () => {
  assert.equal(accept('do   lamp("ку▮', only('ку', 'кухня')), 'do   lamp("кухня"▮')
  assert.equal(accept('do   lamp("▮', only('', 'кухня остров')), 'do   lamp("кухня остров"▮')
})

test('accepting replaces the whole literal and steps over its closing quote', () => {
  assert.equal(
    accept('do   lamp("кух▮ня 2").toggle()', only('кух', 'кухня')),
    'do   lamp("кухня"▮).toggle()',
  )
  assert.equal(accept('do   lamp("кухня▮")', only('кухня', 'кухня')), 'do   lamp("кухня"▮)')
})

test('accepting a name by its id replaces the literal, quotes included', () => {
  const long = only('', 'кухня: подсветка рабочей зоны', 12)
  assert.equal(accept('do   lamp("кухня: подсв▮', long), 'do   lamp(12▮')
  assert.equal(accept('do   lamp("кух▮").on()', long), 'do   lamp(12▮).on()')
  assert.equal(
    accept('when input("панель ▮", inst=1) is short_press', only('', 'панель "прихожая"', 5)),
    'when input(5▮, inst=1) is short_press',
  )
})

test('an input device and a schedule complete like a lamp', () => {
  assert.equal(
    accept('when input("пан▮", inst=1) is short_press', only('пан', 'панель-прихожая')),
    'when input("панель-прихожая"▮, inst=1) is short_press',
  )
  assert.equal(accept('do hcl.disable("▮', only('', 'morning', null)), 'do hcl.disable("morning"▮')
})

test('arrows wrap around the list, Enter and Tab accept, Escape closes', () => {
  const list = { active: 0, count: 3 }
  assert.deepEqual(keyAction(key('ArrowDown'), list), { kind: 'move', active: 1 })
  assert.deepEqual(keyAction(key('ArrowUp'), list), { kind: 'move', active: 2 })
  assert.deepEqual(keyAction(key('ArrowDown'), { active: 2, count: 3 }), { kind: 'move', active: 0 })
  assert.deepEqual(keyAction(key('Enter'), list), { kind: 'accept' })
  assert.deepEqual(keyAction(key('Tab'), list), { kind: 'accept' })
  assert.deepEqual(keyAction(key('Escape'), list), { kind: 'close' })
  assert.equal(keyAction(key('ArrowLeft'), list), null)
  assert.equal(keyAction(key('a'), list), null)
})

test('modified keys and IME composition pass through to the editor', () => {
  const list = { active: 0, count: 3 }
  assert.equal(keyAction(key('Tab', { shiftKey: true }), list), null)
  assert.equal(keyAction(key('Enter', { ctrlKey: true }), list), null)
  assert.equal(keyAction(key('Enter', { metaKey: true }), list), null)
  assert.equal(keyAction(key('ArrowDown', { altKey: true }), list), null)
  assert.equal(keyAction(key('Enter', { isComposing: true }), list), null)
})

test('an empty or closed list takes no keys, and Ctrl+Space opens a closed one', () => {
  assert.equal(keyAction(key('Enter'), { active: 0, count: 0 }), null)
  assert.equal(keyAction(key('Escape'), { active: 0, count: 0 }), null)
  assert.equal(keyAction(key('ArrowDown'), null), null)
  assert.deepEqual(keyAction(key(' ', { ctrlKey: true }), null), { kind: 'open' })
  assert.equal(keyAction(key(' ', { ctrlKey: true }), { active: 0, count: 2 }), null)
})

test('the list opens below its line while there is room', () => {
  const view = { lineHeight: 20, viewWidth: 800, viewHeight: 420 }
  assert.deepEqual(placeList({ ...view, x: 100, lineTop: 40 }), {
    left: 100,
    width: SUGGEST_WIDTH_PX,
    top: 60,
    bottom: null,
    maxHeight: SUGGEST_MAX_HEIGHT_PX,
  })
  assert.deepEqual(placeList({ ...view, x: 100, lineTop: 200 }), {
    left: 100,
    width: SUGGEST_WIDTH_PX,
    top: 220,
    bottom: null,
    maxHeight: 200,
  })
})

test('near the bottom the list opens above its line, capped by the room there', () => {
  const view = { lineHeight: 20, viewWidth: 800, viewHeight: 420 }
  assert.deepEqual(placeList({ ...view, x: 100, lineTop: 380 }), {
    left: 100,
    width: SUGGEST_WIDTH_PX,
    top: null,
    bottom: 40,
    maxHeight: SUGGEST_MAX_HEIGHT_PX,
  })
  assert.equal(placeList({ ...view, viewHeight: 300, x: 0, lineTop: 220 })?.maxHeight, 220)
})

test('the list stays inside the editor, and a line scrolled away shows none', () => {
  const view = { lineHeight: 20, viewWidth: 800, viewHeight: 420 }
  assert.equal(placeList({ ...view, x: 700, lineTop: 40 })?.left, 800 - SUGGEST_WIDTH_PX)
  assert.equal(placeList({ ...view, x: -30, lineTop: 40 })?.left, 0)
  const narrow = placeList({ ...view, viewWidth: 200, x: 150, lineTop: 40 })
  assert.equal(narrow?.width, 200)
  assert.equal(narrow?.left, 0)
  assert.equal(placeList({ ...view, x: 0, lineTop: -20 }), null)
  assert.equal(placeList({ ...view, x: 0, lineTop: 420 }), null)
})

test('the active row scrolls into the list only when it is out of sight', () => {
  const rowHeight = 26
  const viewHeight = 130
  assert.equal(scrollToShow(52, rowHeight, 0, viewHeight), null)
  assert.equal(scrollToShow(130, rowHeight, 0, viewHeight), 26)
  assert.equal(scrollToShow(26, rowHeight, 52, viewHeight), 26)
  assert.equal(scrollToShow(0, rowHeight, 104, viewHeight), 0)
})

test('a line is found by counting breaks before the index', () => {
  assert.deepEqual(lineAt('a\nbc\nd', 4), { line: 1, start: 2 })
  assert.deepEqual(lineAt('\nx', 0), { line: 0, start: 0 })
  assert.deepEqual(lineAt('\nx', 1), { line: 1, start: 1 })
})

test('registry resources become names with the id the language takes in their place', () => {
  const list = registryNames({
    lamps: [{ virtual_lamp_id: 7, name: 'коридор' }],
    groups: [
      { group_id: 3, name: 'ночь' },
      { group_id: 4, name: '' },
    ],
    inputs: [
      { short_address: 5, name: 'панель-прихожая' },
      { short_address: 6, name: null },
    ],
    schedules: [{ schedule_id: 'morning' }],
  })
  assert.deepEqual(list, {
    lamp: [{ name: 'коридор', id: 7 }],
    group: [
      { name: 'ночь', id: 3 },
      { name: '', id: 4 },
    ],
    input: [{ name: 'панель-прихожая', id: 5 }],
    schedule: [{ name: 'morning', id: null }],
  })
  assert.deepEqual(names(rankSuggestions('', list.group)), ['ночь'])
})
