// node flatdiff.mjs <base dir> <new dir> <name>...: which items gained or lost the flat call ABI,
// which representation decisions changed, and the static allocation counters (all, in loops).
import { readFileSync } from 'node:fs'
const [base, neu, ...names] = process.argv.slice(2)
for (const name of names) {
  const a = JSON.parse(readFileSync(`${base}/${name}.js.lane.json`, 'utf8'))
  const b = JSON.parse(readFileSync(`${neu}/${name}.js.lane.json`, 'utf8'))
  const flat = (j) => new Map((j.abi.flat || []).map((e) => [e.item, JSON.stringify(e)]))
  const fa = flat(a), fb = flat(b)
  const gained = [...fb.keys()].filter((k) => !fa.has(k))
  const lost = [...fa.keys()].filter((k) => !fb.has(k))
  const changed = [...fb.keys()].filter((k) => fa.has(k) && fa.get(k) !== fb.get(k))
  const decisions = (j) => new Map((j.repr.decisions || []).map((d) => [JSON.stringify(d.node || d.key || d.item || d), JSON.stringify(d)]))
  const da = decisions(a), db = decisions(b)
  const dchanged = [...db.keys()].filter((k) => da.get(k) !== db.get(k))
  const v = (j) => j.physical && j.physical.final_emitted
  console.log(`== ${name}: allocations ${v(a).allocations} -> ${v(b).allocations}, in loops ${v(a).loop_allocations} -> ${v(b).loop_allocations}`)
  if (gained.length) console.log(`  flat ABI gained: ${gained.join(', ')}`)
  if (lost.length) console.log(`  flat ABI lost: ${lost.join(', ')}`)
  if (changed.length) console.log(`  flat ABI changed: ${changed.join(', ')}`)
  if (dchanged.length) console.log(`  representation decisions changed: ${dchanged.length}: ${dchanged.slice(0, 3).map((k) => db.get(k).slice(0, 160)).join(' | ')}`)
}
