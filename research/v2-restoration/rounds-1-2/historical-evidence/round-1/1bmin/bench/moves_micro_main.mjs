// moves_micro.mjs <shim> <label>: the element-move helpers directly, in the shapes the compiler emits.
import fs from 'node:fs'
import vm from 'node:vm'
const [shim, label] = process.argv.slice(2)
globalThis.console = console
vm.runInThisContext(fs.readFileSync(shim, 'utf8'))
const rt = globalThis.__rt
const N = 4e6
const clone = (e) => ({ x: e.x, y: e.y, z: e.z })
const structs = Array.from({ length: 64 }, (_, i) => ({ x: i, y: i * 2, z: i * 3 }))
const refs = structs.slice()
const ints = Array.from({ length: 64 }, (_, i) => i)
const slots = Array.from({ length: 64 }, (_, i) => ({ buf: ints, off: i }))
// `ptr::swap` as core spells it: a one-element cell, then copy_nonoverlapping, copy, copy_nonoverlapping.
function swaps(buf, c) {
  return () => {
    let n = 0
    for (let i = 0; i < N; i++) {
      const a = i & 63, b = (i * 7) & 63
      const t = [undefined]
      const x = { buf, off: a }, y = { buf, off: b }, tp = { buf: t, off: 0 }
      rt.copy_nonoverlapping(tp, x, 1, c)
      rt.copy(x, y, 1, c)
      rt.copy_nonoverlapping(y, tp, 1, c)
      n += a
    }
    return n
  }
}
// A bulk move of 32 elements between two buffers.
function bulk(src, c) {
  const dst = src.map((e) => (typeof e === 'object' ? clone(e) : e))
  return () => {
    for (let i = 0; i < N / 32; i++) rt.copy_nonoverlapping({ buf: dst, off: 0 }, { buf: src, off: 0 }, 32, c)
    return dst.length
  }
}
function time(name, f) { f(); const t = process.hrtime.bigint(); f(); return `${name}=${(Number(process.hrtime.bigint() - t) / 1e6).toFixed(0)}ms` }
const out = [label]
out.push(time('swap_value', swaps(structs, clone)))
out.push(time('swap_ref', swaps(refs, undefined)))
out.push(time('swap_int', swaps(ints, undefined)))
out.push(time('swap_slot', swaps(slots, undefined)))
out.push(time('bulk_value', bulk(structs, clone)))
out.push(time('bulk_int', bulk(ints, undefined)))
console.log(out.join(' '))
