// node fourway.mjs <name>...: each probe under native (-O3, the verifier's build), base (base
// program + base shim), round 1 (the verifier's compile of round 1 + the round-1 shim), and round 2
// (this worktree's compile + its shim), one line each, and whether each matches native.
import { existsSync, readFileSync } from 'node:fs'
import vm from 'node:vm'
const G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
const V = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/verify-granule-min/probe'
const W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992'
const run = (shim, program) => {
  if (!existsSync(program)) return '(not compiled)'
  const lines = []
  const context = vm.createContext({ console: { log: (...x) => lines.push(x.join(' ')) }, TextDecoder, TextEncoder })
  try {
    vm.runInContext(`${readFileSync(shim, 'utf8')}\n${readFileSync(program, 'utf8')}\nrust_entry();\n`, context)
  } catch (e) {
    lines.push(`THROW: ${e && e.message}`)
  }
  return lines.join(' ')
}
for (const name of process.argv.slice(2)) {
  const native = readFileSync(`${V}/nat/${name}_3.out`, 'utf8').trim().split('\n').join(' ')
  const rows = {
    native,
    base: run(`${G}/base/shim.js`, `${V}/out/base/balanced/${name}.js`),
    round1: run(`${G}/new/shim.js`, `${V}/out/chg/balanced/${name}.js`),
    round2: run(`${W}/runtime/shim.js`, process.env.R2_DIR ? `${process.env.R2_DIR}/${name}.js` : `${G}/probe/r2/${name}.js`),
  }
  console.log(`== ${name}`)
  for (const [k, v] of Object.entries(rows)) console.log(`  ${k.padEnd(7)} ${v === native ? 'ok ' : 'BAD'} ${v.length > 160 ? v.slice(0, 157) + '...' : v}`)
}
