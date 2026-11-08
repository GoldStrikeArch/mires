// JavaScript reference for the fflate-shaped workload: the published `fflate` library (its browser
// ESM build, the `exports` "./browser" import target, in Node too, so every environment runs the
// same code) producing a gzip member of the same string with stored DEFLATE blocks, which is what
// the Rust port (src/fflate.rs `gzip_store`) emits.
// `level: 0` selects fflate's stored-block path; `mtime: 0` makes the header deterministic, matching
// the Rust port's zero MTIME. fflate's header still differs in two bytes (XFL=4, OS=3 against the
// Rust port's XFL=0, OS=255); see tools/timing-protocol.mjs for how the checksum treats them.
import { gzipSync, strToU8 } from '../../../node_modules/fflate/esm/browser.js'

export function run({ text }) {
  return gzipSync(strToU8(text), { level: 0, mtime: 0 })
}
