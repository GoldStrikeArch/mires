import fs from 'node:fs'
const [benchFile, microFile] = process.argv.slice(2)
const med = (xs) => { xs = [...xs].sort((a, b) => a - b); return xs[Math.floor(xs.length / 2)] }
const rows = fs.readFileSync(benchFile, 'utf8').trim().split('\n').map(JSON.parse)
const cells = new Map()
for (const row of rows) {
  const [arm, mode, suite] = row.label.split('|')
  for (const [name, v] of Object.entries(row)) {
    if (name === 'label') continue
    const key = `${mode} ${suite} ${name}`
    if (!cells.has(key)) cells.set(key, { base: [], mine: [], sums: new Set() })
    cells.get(key)[arm].push(v)
    cells.get(key).sums.add(v.checksum)
  }
}
let worst = 0
console.log('benchmark'.padEnd(30), 'n', 'median base/mine (ratio)', '| min base/mine (ratio)')
for (const [key, c] of cells) {
  const bm = med(c.base.map((v) => v.median_ms)), mm = med(c.mine.map((v) => v.median_ms))
  const bn = Math.min(...c.base.map((v) => v.min_ms)), mn = Math.min(...c.mine.map((v) => v.min_ms))
  worst = Math.max(worst, mm / bm, mn / bn)
  console.log(key.padEnd(30), c.base.length + '/' + c.mine.length,
    `${bm.toFixed(2)} / ${mm.toFixed(2)} (${(mm / bm).toFixed(3)})`,
    `| ${bn.toFixed(2)} / ${mn.toFixed(2)} (${(mn / bn).toFixed(3)})`,
    c.sums.size === 1 ? 'checksums agree' : 'CHECKSUM MISMATCH ' + [...c.sums].join(','))
}
const micro = {}
for (const line of fs.readFileSync(microFile, 'utf8').trim().split('\n')) {
  const parts = line.trim().split(' ')
  for (const p of parts.slice(1)) {
    const [name, ms] = p.split('=')
    ;((micro[name] ||= {})[parts[0]] ||= []).push(parseFloat(ms))
  }
}
for (const [name, arms] of Object.entries(micro)) {
  const bm = med(arms.base), mm = med(arms.mine), bn = Math.min(...arms.base), mn = Math.min(...arms.mine)
  worst = Math.max(worst, mm / bm, mn / bn)
  console.log(('micro ' + name).padEnd(30), arms.base.length + '/' + arms.mine.length,
    `${bm.toFixed(0)} / ${mm.toFixed(0)} (${(mm / bm).toFixed(3)})`, `| ${bn.toFixed(0)} / ${mn.toFixed(0)} (${(mn / bn).toFixed(3)})`)
}
console.log('worst ratio', worst.toFixed(3))
