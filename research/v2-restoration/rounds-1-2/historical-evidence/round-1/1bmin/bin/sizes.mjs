// sizes.mjs <baseDir> <mineDir>: raw/min/gzip sizes of the compact package artifacts in two dists.
import fs from 'node:fs'
import zlib from 'node:zlib'
const [a, b] = process.argv.slice(2)
const files = ['fp.esm.js', 'fp.esm.min.js', 'fp.min.js', 'fp.umd.min.js', 'raw/program.js', 'raw/shim.js']
console.log('file'.padEnd(18), 'base raw/gzip'.padStart(20), 'mine raw/gzip'.padStart(20), 'delta raw/gzip')
for (const f of files) {
  const x = fs.readFileSync(`${a}/${f}`), y = fs.readFileSync(`${b}/${f}`)
  const gx = zlib.gzipSync(x, { level: 9 }).length, gy = zlib.gzipSync(y, { level: 9 }).length
  console.log(f.padEnd(18), `${x.length}/${gx}`.padStart(20), `${y.length}/${gy}`.padStart(20), `${y.length - x.length}/${gy - gx}`,
    Buffer.compare(x, y) === 0 ? 'identical' : '')
}
