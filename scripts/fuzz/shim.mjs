#!/usr/bin/env node
//
// The LLVM side of the fuzzer & metamorphic oracle: runs a `wasm32-unknown-unknown` build of a
// program and prints exactly what runtime/shim.js prints for the rustc_codegen_mires build of the
// same source.
//
//   node scripts/fuzz/shim.mjs <program.wasm>
//
// The imports are the `extern "C"` names scripts/fuzz/gen.mjs and examples/core-tests/prelude.rs
// declare, which are in turn the ones runtime/shim.js installs on `__rt` — that shared vocabulary
// is what lets one source file be the input to both compilers. `print_*` aliases are installed
// beside them so a hand-written probe can use either spelling.
//
// PRINTING HAS TO MATCH THE OTHER SIDE EXACTLY
// --------------------------------------------
// Both sides end up in the same `console.log`, so the only thing that can differ is the JavaScript
// VALUE handed to it, and each import converts to whatever the JS backend would have passed:
//
//   js_log_i32   an i32 arrives as a JS number             -> console.log(x)
//   js_log_i64   an i64 arrives as a BigInt                -> console.log(x.toString()), bare
//   js_log_f64   an f64 arrives as a JS number             -> console.log(x)
//   js_log_bool  a bool arrives as 0 or 1                  -> console.log(x !== 0)
//   js_log_str   a &str is a fat pointer into the memory   -> console.log(text)
//
// The C ABI for `wasm32-unknown-unknown` passes a `&str` INDIRECTLY: the import receives one i32
// pointing at a { ptr: u32, len: u32 } pair in linear memory. Both that shape and a two-argument
// (ptr, len) lowering are accepted here, because which one a given rustc picks is an ABI detail
// rather than something this fuzzer is testing.
//
// An abort exits 101 and prints nothing: the JS side throws instead, so the two runs are compared
// on "did it end non-zero", not on the number.

import { readFileSync } from 'node:fs'

const path = process.argv[2]
if (!path) {
  console.error('usage: shim.mjs <program.wasm>')
  process.exit(2)
}

let memory = null

const readString = (...args) => {
  const view = new DataView(memory.buffer)
  let pointer
  let length
  if (args.length >= 2) {
    pointer = args[0] >>> 0
    length = args[1] >>> 0
  } else {
    const slot = args[0] >>> 0
    pointer = view.getUint32(slot, true)
    length = view.getUint32(slot + 4, true)
  }
  if (length === 0) return ''
  return Buffer.from(new Uint8Array(memory.buffer, pointer, length)).toString('utf8')
}

const abort = () => {
  process.exit(101)
}

const host = {
  js_log_i32: (x) => console.log(x),
  js_log_i64: (x) => console.log(x.toString()),
  js_log_f64: (x) => console.log(x),
  js_log_bool: (x) => console.log(x !== 0),
  js_log_str: (...args) => console.log(readString(...args)),
  js_abort: abort,
}
host.print_i32 = host.js_log_i32
host.print_i64 = host.js_log_i64
host.print_f64 = host.js_log_f64
host.print_bool = host.js_log_bool
host.print_str = host.js_log_str
host.print_bytes = (pointer, length) => console.log(readString(pointer, length))
host.abort = abort

const { instance } = await WebAssembly.instantiate(readFileSync(path), { env: host })
memory = instance.exports.memory
if (typeof instance.exports.rust_entry !== 'function') {
  console.error('shim.mjs: the module exports no rust_entry')
  process.exit(2)
}
try {
  instance.exports.rust_entry()
} catch (error) {
  // A trap — an `unreachable` from a panic that reached no handler, or an out of bounds access.
  // The JS side ends non-zero for the same program, so this does too.
  console.error(`shim.mjs: trapped: ${error}`)
  process.exit(101)
}
