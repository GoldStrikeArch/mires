// node dbg.mjs <shim> <program>: traces the first conversions and address queries of a program.
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
const [shim, program] = process.argv.slice(2)
const lines = []
const context = vm.createContext({ console: { log: (...x) => lines.push(x.join(' ')) }, TextDecoder, TextEncoder })
vm.runInContext(readFileSync(shim, 'utf8'), context)
const rt = context.__rt
let n = 0
const orig = rt.convert_block_str
rt.convert_block_str = (buf, off, len) => {
  const had = rt._addrs.has(buf)
  const heap = rt._addrHeap.has(buf)
  globalThis.lastBuf = buf; const r = orig(buf, off, len)
  if (n++ < 3) lines.push(`convert_block_str: had addr ${had} heap ${heap} -> after addr ${rt._addrs.has(buf)} isBlock ${rt._blocks.has(buf)} len ${buf && buf.length}`)
  return r
}
const origAddr = rt.addr
let m = 0
const kind = (p) => {
  if (p === null || typeof p !== 'object') return typeof p
  if (rt._blocks.has(p.buf)) return 'block'
  const view = rt._scalar_view_of.get(p.buf)
  if (view !== undefined) return `scalar-view(of block: ${rt._blocks.has(view.buf)})`
  return `other:${p.buf && p.buf.constructor && p.buf.constructor.name}`
}
rt.addr = (p, size) => {
  const a = origAddr(p, size)
  if (n > 0 && n < 3 && m++ < 12) lines.push(`addr(${kind(p)}, ${size}) = ${a} key-has=${rt._addrs.has(p.buf)} buf-is-last=${p.buf === globalThis.lastBuf}`)
  return a
}
try {
  vm.runInContext(`${readFileSync(program, 'utf8')}\nrust_entry();`, context)
} catch (e) {
  lines.push(`THROW ${e.message}`)
}
console.log(lines.slice(0, 60).join('\n'))
