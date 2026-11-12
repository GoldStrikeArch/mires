// node run.mjs <shim.js> <program.js>: shim + program + rust_entry(), as scripts/test.sh and
// scripts/check-known-gaps.py run a compiled core program.
import { readFileSync } from 'node:fs'
import vm from 'node:vm'
const [shim, program] = process.argv.slice(2)
const context = vm.createContext({ console, TextDecoder, TextEncoder })
try {
  vm.runInContext(`${readFileSync(shim, 'utf8')}\n${readFileSync(program, 'utf8')}\nrust_entry();\n`, context)
} catch (e) {
  console.log(`THROW: ${e && e.message}`)
  process.exitCode = 3
}
