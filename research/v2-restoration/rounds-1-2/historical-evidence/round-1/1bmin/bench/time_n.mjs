import fs from 'node:fs'
import vm from 'node:vm'
const [program, shim, label, ...names] = process.argv.slice(2)
const context = vm.createContext({ TextDecoder, TextEncoder, console })
vm.runInContext(fs.readFileSync(shim, 'utf8'), context)
vm.runInContext(fs.readFileSync(program, 'utf8'), context)
const N = Number(process.env.BENCH_N || 1_000_000)
const out = {}
for (const name of names) {
  const f = context[name]
  f(100_000); f(100_000) // warm up
  const times = []
  let checksum = 0
  for (let i = 0; i < 5; i++) { const t = process.hrtime.bigint(); checksum = f(N); times.push(Number(process.hrtime.bigint() - t) / 1e6) }
  times.sort((a, b) => a - b)
  out[name] = { median_ms: times[2], min_ms: times[0], checksum }
}
console.log(JSON.stringify({ label, ...out }))
