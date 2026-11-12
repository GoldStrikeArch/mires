// node rerun.mjs <shim.js> <compiled dir> <tests dir>: runs every compiled core test in <compiled
// dir> under <shim.js> exactly as scripts/test.sh does (shim + program + `rust_entry();` on node's
// stdin), and compares stdout with the fixture's .expected (an .expect_abort test must exit non-zero).
// Prints one line per mismatch and a summary, plus how many blocks the shim poisoned if it counts.
import { spawnSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
const [shimPath, compiled, tests] = process.argv.slice(2)
const shim = readFileSync(shimPath, 'utf8')
let pass = 0, fail = 0, skipped = 0
for (const name of readdirSync(tests).filter((n) => n.endsWith('.rs')).map((n) => n.slice(0, -3)).sort()) {
  const program = join(compiled, `${name}.js`)
  const expected = join(tests, `${name}.expected`)
  if (!existsSync(program) || !existsSync(expected)) { skipped++; continue }
  const abort = existsSync(join(tests, `${name}.expect_abort`))
  const tail = '\nrust_entry();\nif (globalThis.__rt._poisoned) console.error("POISONED " + globalThis.__rt._poisoned);\n'
  const run = spawnSync(process.execPath, ['-'], { input: shim + '\n' + readFileSync(program, 'utf8') + tail, encoding: 'utf8', maxBuffer: 1 << 28 })
  const ok = (abort ? run.status !== 0 : run.status === 0) && run.stdout === readFileSync(expected, 'utf8')
  const poisoned = /POISONED (\d+)/.exec(run.stderr || '')
  if (ok) pass++
  else { fail++; console.log(`FAIL ${name} status=${run.status} ${(run.stderr || '').split('\n').filter(Boolean).slice(-2).join(' | ')}`) }
  if (poisoned) console.log(`poisoned ${name}: ${poisoned[1]} consumed blocks`)
}
console.log(JSON.stringify({ pass, fail, skipped }))
