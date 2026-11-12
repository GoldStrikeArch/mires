// node sites.mjs <file>...: prints every consuming-conversion site (convert_block_nv, or a decode
// over a String's `.vec` leaf) with what consumes the converted value right after it.
import { readFileSync } from 'node:fs'
const re = /(?:([A-Za-z_$][\w$]*)=)?(\$[\w$]*\()?(\{text:)?__rt\.(convert_block_nv|convert_block_str|bytes_str|drop_block_nv|drop_block_str)\(([^,()]*)/g
for (const file of process.argv.slice(2)) {
  const text = readFileSync(file, 'utf8')
  for (const m of text.matchAll(re)) {
    if (/str$/.test(m[4]) && !/\.vec\./.test(m[5])) continue // a read-only decode, not an adapter
    const at = m.index
    const fnStart = text.lastIndexOf('function ', at)
    const fnName = /function ([^(]*)/.exec(text.slice(fnStart, fnStart + 60))[1]
    // the statement after the conversion: what receives the converted value
    const close = text.indexOf(';', at + m[0].length)
    const after = text.slice(close + 1, close + 60).replace(/\n/g, ' ')
    const where = m[2] ? `passed directly to ${m[2].slice(0, -1)}` : m[1] ? `bound to ${m[1]}, then: ${after}` : `inline: ${text.slice(at - 30, at + 10)}`
    const kind = m[3] ? 'builder' : /nv$/.test(m[4]) ? 'vec' : 'host'
    const releases = /^drop_block|^convert_block_str/.test(m[4]) ? ' RELEASING' : ''
    console.log(`${file.split('/').pop()} ${fnName}: ${kind}${releases} ${where}`)
  }
}
