// micro_report.mjs <micro-ab.txt> <baseArm> <arm...>: median and min per micro and arm, as ratios to the base arm.
import fs from 'node:fs'
const [file, base, ...arms] = process.argv.slice(2)
const med = (xs) => { xs = [...xs].sort((a, b) => a - b); return xs[Math.floor(xs.length / 2)] }
const micro = {}
for (const line of fs.readFileSync(file, 'utf8').trim().split('\n')) {
  const parts = line.trim().split(' ')
  for (const p of parts.slice(1)) {
    const [name, ms] = p.split('=')
    ;((micro[name] ||= {})[parts[0]] ||= []).push(parseFloat(ms))
  }
}
for (const [name, byArm] of Object.entries(micro)) {
  const b = byArm[base]
  const cells = arms.map((a) => {
    const x = byArm[a]
    return `${a} med ${med(x).toFixed(0)} (${(med(x) / med(b)).toFixed(3)}) min ${Math.min(...x).toFixed(0)} (${(Math.min(...x) / Math.min(...b)).toFixed(3)})`
  })
  console.log(name.padEnd(14), `n=${b.length}`, `${base} med ${med(b).toFixed(0)} min ${Math.min(...b).toFixed(0)}`, '|', cells.join(' | '))
}
