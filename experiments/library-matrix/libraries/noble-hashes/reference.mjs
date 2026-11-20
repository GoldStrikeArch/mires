import { sha256, sha512 } from '../../node_modules/@noble/hashes/sha2.js'
import { sha3_256, keccak_256 } from '../../node_modules/@noble/hashes/sha3.js'
import { blake2b, blake2s } from '../../node_modules/@noble/hashes/blake2.js'
import { hmac } from '../../node_modules/@noble/hashes/hmac.js'
import { bytesToHex } from '../../node_modules/@noble/hashes/utils.js'

const hashes = { sha256, sha512, sha3_256, keccak_256, blake2b, blake2s }

export function run(input) {
  const { algorithm, bytes, chunks, repeat = 1 } = input
  const isHmac = algorithm === 'hmac_sha256'
  if (!isHmac && !Object.hasOwn(hashes, algorithm)) throw new TypeError('unknown hash')
  const options = {}
  for (const k of ['key', 'salt', 'personalization']) {
    if (input[k] !== undefined) options[k] = Uint8Array.from(input[k])
  }
  if (input.dkLen !== undefined) options.dkLen = input.dkLen
  if (bytes !== undefined) {
    const message = Uint8Array.from(bytes)
    return bytesToHex(isHmac ? hmac(sha256, options.key, message) : hashes[algorithm](message, options))
  }
  const state = isHmac ? hmac.create(sha256, options.key) : hashes[algorithm].create(options)
  const blocks = chunks.map(chunk => Uint8Array.from(chunk))
  for (let i = 0; i < repeat; i++) {
    for (const block of blocks) state.update(block)
  }
  return bytesToHex(state.digest())
}
