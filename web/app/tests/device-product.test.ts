import assert from 'node:assert/strict'
import test from 'node:test'

import { productGtin } from '../src/screens/device-product.js'

const list = [
  { short_address: 4, gtin: 4_012_345_678_901 },
  { short_address: 5, gtin: null },
]

test('the overview names the product from the device list it already polls', () => {
  assert.equal(productGtin(null, list, 4), 4_012_345_678_901)
})

test('a GTIN read from the identity bank wins over the list', () => {
  assert.equal(productGtin(7_000_000_000_001, list, 4), 7_000_000_000_001)
})

test('a device with no GTIN anywhere names no product', () => {
  assert.equal(productGtin(null, list, 5), null)
  assert.equal(productGtin(null, undefined, 4), null)
})
