// node count.mjs <shim> <program>: runs the program and reports how many consuming conversions
// (convert_block_nv / convert_block_str, or the base's builder decode over a `.vec` block) ran
// before it finished or aborted, and the bump pointer at the end.
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
const [shim, program] = process.argv.slice(2)
const lines = []
const context = vm.createContext({ console: { log: (...x) => lines.push(x.join(' ')) }, TextDecoder, TextEncoder })
vm.runInContext(readFileSync(shim, 'utf8'), context)
const rt = context.__rt
const counts = { nv: 0, str: 0, bytes_str: 0 }
const wrap = (name, key) => {
  const original = rt[name]
  if (original === undefined) return
  rt[name] = (...args) => { counts[key]++; return original(...args) }
}
wrap('convert_block_nv', 'nv')
wrap('convert_block_str', 'str')
wrap('bytes_str', 'bytes_str')
let error = null
try {
  vm.runInContext(`${readFileSync(program, 'utf8')}\nrust_entry();`, context)
} catch (e) {
  error = e && e.message
}
console.log(JSON.stringify({ output: lines, error, conversions: counts, addrNext: rt._addrNext }))
