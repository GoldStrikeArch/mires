// time_cpu.mjs <program> <shim> <label> <names...>: like time2.mjs, but every sample is the
// process's CPU time (user + system) across the call, so the time the process spends descheduled on
// a loaded machine is not counted. Wall time is recorded beside it.
import fs from 'node:fs'
import vm from 'node:vm'
const [program, shim, label, ...names] = process.argv.slice(2)
const context = vm.createContext({ TextDecoder, TextEncoder, console })
vm.runInContext(fs.readFileSync(shim, 'utf8'), context)
vm.runInContext(fs.readFileSync(program, 'utf8'), context)
const N = Number(process.env.BENCH_N || 50_000)
const out = {}
for (const name of names) {
  const f = context[name]
  f(N / 2); f(N / 2) // warm up
  const cpu = []
  const wall = []
  let checksum = 0
  for (let i = 0; i < 5; i++) {
    const c = process.cpuUsage()
    const t = process.hrtime.bigint()
    checksum = f(N)
    wall.push(Number(process.hrtime.bigint() - t) / 1e6)
    const d = process.cpuUsage(c)
    cpu.push((d.user + d.system) / 1e3)
  }
  cpu.sort((a, b) => a - b)
  wall.sort((a, b) => a - b)
  out[name] = { median_ms: cpu[2], min_ms: cpu[0], wall_median_ms: wall[2], checksum }
}
console.log(JSON.stringify({ label, ...out }))
