// node samediff.mjs <base dir> <new dir> [ext]: for every program in both directories, whether the
// new one is byte-identical to the base one, identical once the consuming decode is spelled back
// as the base's (`__rt.convert_block_str(` -> `__rt.bytes_str(`), or different.
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
const [base, neu, ext = '.js'] = process.argv.slice(2)
const out = { identical: [], renameOnly: [], different: [], missing: [] }
for (const name of readdirSync(base).filter((n) => n.endsWith(ext)).sort()) {
  const b = join(base, name), n = join(neu, name)
  if (!existsSync(n)) { out.missing.push(name); continue }
  const x = readFileSync(b, 'utf8'), y = readFileSync(n, 'utf8')
  const renamed = y.split('__rt.convert_block_str(').join('__rt.bytes_str(')
    .split('__rt.drop_block_str(').join('__rt.bytes_str(')
    .split('__rt.drop_block_nv(').join('__rt.convert_block_nv(')
  if (x === y) out.identical.push(name)
  else if (x === renamed) out.renameOnly.push(name)
  else out.different.push(name)
}
console.log(JSON.stringify({ identical: out.identical.length, renameOnly: out.renameOnly, different: out.different, missing: out.missing }))
