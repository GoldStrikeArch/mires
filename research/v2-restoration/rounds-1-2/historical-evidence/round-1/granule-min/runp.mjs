// node runp.mjs <shim> <program>...: like probe/run.mjs, one fresh context per program; prints each
// program's output on one line, and how many blocks a poisoning shim poisoned.
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
const [shim, ...programs] = process.argv.slice(2)
const shimSource = readFileSync(shim, 'utf8')
for (const program of programs) {
  const lines = []
  const out = { log: (...xs) => lines.push(xs.join(' ')) }
  const context = vm.createContext({ console: out, TextDecoder, TextEncoder })
  try {
    vm.runInContext(`${shimSource}\n${readFileSync(program, 'utf8')}\nrust_entry();\n`, context)
  } catch (e) {
    lines.push(`THROW: ${e && e.message}`)
  }
  const poisoned = context.__rt._poisoned ? ` (poisoned ${context.__rt._poisoned})` : ''
  console.log(`${program.split('/').pop()}: ${lines.join(' ')}${poisoned}`)
}
