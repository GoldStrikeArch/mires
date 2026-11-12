// node wlcmp.mjs: per browser workload, whether the base and new builds' program, minimal shim and
// shipped index.min.js are identical (or differ only by the consuming-decode rename), with sizes.
import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { brotliCompressSync, constants, gzipSync } from 'node:zlib'
const LABEL = process.argv[2] || 'new'
const G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
const size = (data) => ({
  bytes: data.length,
  gzip: gzipSync(data, { level: 9 }).length,
  brotli: brotliCompressSync(data, { params: { [constants.BROTLI_PARAM_QUALITY]: 11, [constants.BROTLI_PARAM_LGWIN]: 22, [constants.BROTLI_PARAM_SIZE_HINT]: data.length } }).length,
})
const unrename = (b) => b.split('__rt.convert_block_str(').join('__rt.bytes_str(')
  .split('__rt.drop_block_str(').join('__rt.bytes_str(').split('__rt.drop_block_nv(').join('__rt.convert_block_nv(')
const compare = (a, b) => a === b ? 'same' : a === unrename(b) ? 'rename-only' : 'DIFFERENT'
for (const w of readdirSync(`${G}/base/wl-dist`)) {
  const read = (which, path) => existsSync(`${G}/${which}/wl-dist/${w}/${path}`) ? readFileSync(`${G}/${which}/wl-dist/${w}/${path}`, 'utf8') : null
  const row = { workload: w }
  for (const path of ['raw/program.js', 'raw/shim.js', 'index.min.js']) {
    const a = read('base', path), b = read(LABEL, path)
    row[path] = a === null || b === null ? 'absent' : compare(a, b)
  }
  const a = read('base', 'index.min.js'), b = read(LABEL, 'index.min.js')
  if (a !== null && b !== null) { row.base = size(Buffer.from(a)); row.new = size(Buffer.from(b)) }
  console.log(JSON.stringify(row))
}
