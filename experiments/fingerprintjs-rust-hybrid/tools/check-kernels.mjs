#!/usr/bin/env node

import { murmurX64Hash128 } from '../js/hash.js'
import { componentsToCanonicalString } from '../js/canonical.js'

const hashCases = [
  ['Hello, world, hi', '9a66b4567d520770dc8eaf9a508ecf1b'],
  ['Hello, world, h', '4552c6409e0a7bd3b0f9eb318bb35f05'],
  ['Hello, world, hi!', 'dce3e02d43da4d2374e84e484c566492'],
  ['Hello, world, hi, Hello, world, hi', 'd49c261c833b671870b471c42df4dbf0'],
  ['hello', 'cbd8a7b341bd9b025b1e906a48ae1d19'],
]

let failed = 0
for (const [input, expected] of hashCases) {
  const got = murmurX64Hash128(input, 0)
  if (got !== expected) {
    failed += 1
    console.error(`murmur FAIL ${JSON.stringify(input)}: ${got} != ${expected}`)
  }
}

const nonAscii = [...'Hello, world, hi']
  .map((character) => String.fromCharCode(character.charCodeAt(0) + 256))
  .join('')
const nonAsciiGot = murmurX64Hash128(nonAscii, 0)
if (nonAsciiGot !== '460892d2cab76edff07f62a97e106f6b') {
  failed += 1
  console.error(`murmur FAIL non-ascii: ${nonAsciiGot}`)
}

const unpaired = murmurX64Hash128('a\uD800b', 0)
const replaced = murmurX64Hash128('a\uFFFDb', 0)
if (unpaired !== replaced) {
  failed += 1
  console.error(`murmur FAIL unpaired surrogate: ${unpaired} != ${replaced}`)
}

const canonical = componentsToCanonicalString({
  z: { value: undefined },
  'b|c:d\\e': { value: [1, true] },
  a: { error: new Error('x') },
})
const expectedCanonical = 'a:error|b\\|c\\:d\\\\e:[1,true]|z:undefined'
if (canonical !== expectedCanonical) {
  failed += 1
  console.error(`canonical FAIL: ${JSON.stringify(canonical)} != ${JSON.stringify(expectedCanonical)}`)
}

if (failed) {
  process.exit(1)
}
console.log('js kernels: murmur and canonical vectors ok')
