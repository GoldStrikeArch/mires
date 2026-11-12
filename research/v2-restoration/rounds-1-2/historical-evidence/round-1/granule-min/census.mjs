// node census.mjs <dir|file>...: per program, how many sites take an address and how many
// conversions consume a baseline block (vector, string, builder), by spelling.
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
const files = []
for (const arg of process.argv.slice(2)) {
  if (statSync(arg).isDirectory()) {
    for (const name of readdirSync(arg)) if (name.endsWith('.js') && !name.endsWith('.jir.js')) files.push(join(arg, name))
  } else files.push(arg)
}
const count = (text, re) => (text.match(re) || []).length
let totals = { addr: 0, nv: 0, str: 0, builder: 0, progs: 0, withConv: 0 }
for (const file of files.sort()) {
  const text = readFileSync(file, 'utf8')
  const row = {
    addr: count(text, /__rt\.addr\(/g),
    nv: count(text, /__rt\.convert_block_nv\(/g),
    // a String adapter: the decode over a String's `vec...ptr` leaf, builder or host
    builder: count(text, /text:\s*__rt\.(?:bytes_str|convert_block_str)\([^,]*\.vec\./g),
    str: count(text, /__rt\.(?:bytes_str|convert_block_str)\([^,]*\.vec\./g),
    plainDecodes: count(text, /__rt\.bytes_str\(/g),
    consumingDecodes: count(text, /__rt\.convert_block_str\(/g),
  }
  row.str -= row.builder
  totals.progs++
  if (row.nv || row.str || row.builder) {
    totals.withConv++
    console.log(`${file.split('/').pop()}: addr=${row.addr} nv=${row.nv} host=${row.str} builder=${row.builder} bytes_str=${row.plainDecodes} convert_block_str=${row.consumingDecodes}`)
  }
  totals.addr += row.addr > 0 ? 1 : 0
  totals.nv += row.nv; totals.str += row.str; totals.builder += row.builder
}
console.log(JSON.stringify(totals))
