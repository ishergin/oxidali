import assert from 'node:assert/strict'
import test from 'node:test'

import { rgbwafControlType, rgbwafDrives, rgbwafIsTarget } from '../src/format.js'

test('bits 7:6 name the control type the way DiiA 098bp does', () => {
  assert.equal(rgbwafControlType(0x80), 'normalised colour control')
  assert.equal(rgbwafControlType(0xc0), 'extended colour control')
})

test('the two values 098bp reserves keep their 209:2011 names in brackets', () => {
  assert.equal(rgbwafControlType(0x00), 'reserved (209:2011 channel control)')
  assert.equal(rgbwafControlType(0x3f), 'reserved (209:2011 channel control)')
  assert.equal(rgbwafControlType(0x40), 'reserved (209:2011 colour control)')
})

test('the linked channels do not change the control type', () => {
  assert.equal(rgbwafControlType(0x87), 'normalised colour control')
  assert.equal(rgbwafControlType(0xff), 'extended colour control')
})

test('extended colour control with unlinked levels drives the colour but is not the asserted value', () => {
  assert.equal(rgbwafDrives(0xc0), true)
  assert.equal(rgbwafIsTarget(0xc0), false)
  assert.equal(rgbwafIsTarget(0x80), true)
})
