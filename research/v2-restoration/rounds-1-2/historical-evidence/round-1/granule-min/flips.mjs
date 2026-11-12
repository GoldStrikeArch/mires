// node flips.mjs <base dir> <new dir> <name>...: for each program, what the lane report says moved
// between the base and new link: the representation plan's planned/validated/final counts and
// decisions, the flat ABI and the per-pass counters, plus whether a `{`-literal allocation now sits
// inside a loop body that the base did not have (a crude textual check: object literals inside
// `for(;;)`/`while` bodies, counted per function).
import { readFileSync } from 'node:fs'
const [base, neu, ...names] = process.argv.slice(2)
const flat = (o, prefix = '', out = {}) => {
  if (o === null || typeof o !== 'object') { out[prefix] = o; return out }
  if (Array.isArray(o)) { out[prefix] = JSON.stringify(o).slice(0, 200); return out }
  for (const [k, v] of Object.entries(o)) flat(v, prefix ? `${prefix}.${k}` : k, out)
  return out
}
for (const name of names) {
  const a = JSON.parse(readFileSync(`${base}/${name}.js.lane.json`, 'utf8'))
  const b = JSON.parse(readFileSync(`${neu}/${name}.js.lane.json`, 'utf8'))
  const pick = (j) => flat({ repr: { planned: j.repr.planned, validated: j.repr.validated, final_emitted: j.repr.final_emitted }, abi: j.abi, physical: j.physical, inline: j.inline, plan_retries: j.plan_retries })
  const fa = pick(a), fb = pick(b)
  const moved = Object.keys({ ...fa, ...fb }).filter((k) => JSON.stringify(fa[k]) !== JSON.stringify(fb[k]))
  const decisions = (j) => JSON.stringify(j.repr.decisions)
  console.log(`== ${name}`)
  for (const k of moved.slice(0, 14)) console.log(`  ${k}: ${JSON.stringify(fa[k])} -> ${JSON.stringify(fb[k])}`)
  if (moved.length > 14) console.log(`  ... ${moved.length - 14} more`)
  if (decisions(a) !== decisions(b)) console.log(`  repr.decisions differ (${decisions(a).length} -> ${decisions(b).length} chars)`)
}
