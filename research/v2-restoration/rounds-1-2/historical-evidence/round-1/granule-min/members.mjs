import { readFileSync } from 'node:fs'
const { shimMembers } = await import(process.argv[2] + '/scripts/minimal-shim.mjs')
const a = shimMembers(readFileSync(process.argv[3], 'utf8')), b = shimMembers(readFileSync(process.argv[4], 'utf8'))
for (const name of new Set([...a.keys(), ...b.keys()])) {
  const x = a.get(name), y = b.get(name)
  if (x !== y) console.log(`${name}: ${x === undefined ? 'absent' : x.length} -> ${y === undefined ? 'absent' : y.length}`)
}
