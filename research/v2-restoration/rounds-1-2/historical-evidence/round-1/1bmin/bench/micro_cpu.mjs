// micro_cpu.mjs <shim> <label>: ptr_micro.mjs's three loops and moves_micro.mjs's six, each timed in
// process CPU time (user + system), minimum of three runs after a warm-up.
import fs from 'node:fs'
import vm from 'node:vm'
const [shim, label] = process.argv.slice(2)
const ctx = vm.createContext({ console })
vm.runInContext(fs.readFileSync(shim, 'utf8'), ctx)
const rt = ctx.__rt
const buf = new Array(1000).fill(0).map((_, i) => i)
const M = 1e6
const N = 1e5
const clone = (e) => ({ x: e.x, y: e.y, z: e.z })
const structs = Array.from({ length: 64 }, (_, i) => ({ x: i, y: i * 2, z: i * 3 }))
const refs = structs.slice()
const ints = Array.from({ length: 64 }, (_, i) => i)
const slots = Array.from({ length: 64 }, (_, i) => ({ buf: ints, off: i }))
function swaps(b, c) {
  return () => {
    let n = 0
    for (let i = 0; i < N; i++) {
      const a = i & 63, d = (i * 7) & 63
      const t = [undefined]
      const x = { buf: b, off: a }, y = { buf: b, off: d }, tp = { buf: t, off: 0 }
      rt.copy_nonoverlapping(tp, x, 1, c)
      rt.copy(x, y, 1, c)
      rt.copy_nonoverlapping(y, tp, 1, c)
      n += a
    }
    return n
  }
}
function bulk(src, c) {
  const dst = src.map((e) => (typeof e === 'object' ? clone(e) : e))
  return () => {
    for (let i = 0; i < N / 32; i++) rt.copy_nonoverlapping({ buf: dst, off: 0 }, { buf: src, off: 0 }, 32, c)
    return dst.length
  }
}
const loops = {
  ptr_eq_elem: () => { let n = 0; for (let i = 0; i < M; i++) { const a = { buf, off: i & 511 }, b = { buf, off: 512 }; if (rt.ptr_eq(a, b)) n++ } return n },
  ptr_cmp_elem: () => { let n = 0; for (let i = 0; i < M; i++) { const a = { buf, off: i & 511 }, b = { buf, off: 256 }; n += rt.ptr_cmp(a, b, 1) } return n },
  addr_elem: () => { let n = 0; for (let i = 0; i < M; i++) { n += rt.addr({ buf, off: i & 511 }, 4) } return n },
  swap_value: swaps(structs, clone),
  swap_ref: swaps(refs, undefined),
  swap_int: swaps(ints, undefined),
  swap_slot: swaps(slots, undefined),
  bulk_value: bulk(structs, clone),
  bulk_int: bulk(ints, undefined),
}
const out = [label]
for (const [name, f] of Object.entries(loops)) {
  f()
  let best = Infinity
  for (let k = 0; k < 3; k++) {
    const c = process.cpuUsage()
    f()
    const d = process.cpuUsage(c)
    best = Math.min(best, (d.user + d.system) / 1e3)
  }
  out.push(`${name}=${best.toFixed(0)}ms`)
}
console.log(out.join(' '))
