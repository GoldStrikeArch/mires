// node sizes.mjs <file>...: bytes, gzip level 9 and brotli quality 11 / lgwin 22 (Node zlib only).
import { readFileSync } from 'node:fs'
import { brotliCompressSync, constants, gzipSync } from 'node:zlib'
for (const file of process.argv.slice(2)) {
  const data = readFileSync(file)
  const gzip = gzipSync(data, { level: 9 }).length
  const brotli = brotliCompressSync(data, {
    params: {
      [constants.BROTLI_PARAM_QUALITY]: 11,
      [constants.BROTLI_PARAM_LGWIN]: 22,
      [constants.BROTLI_PARAM_SIZE_HINT]: data.length,
    },
  }).length
  console.log(JSON.stringify({ file: file.split('/').slice(-3).join('/'), bytes: data.length, gzip, brotli }))
}
