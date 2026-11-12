// node ab.mjs <rounds> [--samples N]: paired A/B of the three timed browser workloads with
// `run.mjs --pure`, base dist against new dist, the order alternating every round. Each run's
// rust-vs-js cell gives the Rust arm's p50; its rust-vs-rust cell is the A/A control (rust-a/rust-b
// p50, same artifact twice). Writes every run's JSON to timing/ and prints the summary.
import { execFileSync } from 'node:child_process'
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import os from 'node:os'
const G = '/private/tmp/claude-501/-Users-mikhail-pertsev-Desktop-OSS-mires/0488a7b9-01d1-4964-879d-71c26c099cf1/scratchpad/granule-min'
const W = '/Users/mikhail.pertsev/Desktop/OSS/mires/.claude/worktrees/agent-ac96e6c4714b6a992'
const rounds = Number(process.argv[2] || 6)
const extra = process.argv.slice(3)
const NEW = process.env.NEW_LABEL || "new"
const T = `${G}/timing-${NEW}`
mkdirSync(T, { recursive: true })
const dirOf = (which) => (which === "new" ? NEW : which)
const install = (which) => {
  rmSync(`${W}/experiments/browser-workloads/dist`, { recursive: true, force: true })
  cpSync(`${G}/${dirOf(which)}/wl-dist`, `${W}/experiments/browser-workloads/dist`, { recursive: true })
  mkdirSync(`${W}/experiments/browser-workloads/results`, { recursive: true })
  cpSync(`${G}/${dirOf(which)}/wl-build.json`, `${W}/experiments/browser-workloads/results/build.json`)
}
const runs = { base: [], new: [] }
for (let r = 0; r < rounds; r++) {
  for (const which of r % 2 ? ['new', 'base'] : ['base', 'new']) {
    install(which)
    const output = `${T}/${which}-${r}.json`
    const load = os.loadavg()[0]
    execFileSync(process.execPath, ['experiments/browser-workloads/tools/run.mjs', '--pure', '--output', output, ...extra],
      { cwd: W, stdio: ['ignore', 'ignore', 'inherit'], env: { ...process.env, NODE_OPTIONS: '--preserve-symlinks' } })
    const rows = JSON.parse(readFileSync(output, 'utf8')).timing.rows
    const p50 = (workload, pair, arm) => rows.find((x) => x.workload === workload && x.pair === pair && x.arm === arm).p50
    const cell = {}
    for (const w of ['marked', 'minisearch', 'fflate']) {
      cell[w] = { rust: p50(w, 'rust-vs-js', 'rust'), a: p50(w, 'rust-vs-rust', 'rust-a'), b: p50(w, 'rust-vs-rust', 'rust-b'),
        checksumOk: rows.filter((x) => x.workload === w).every((x) => x.checksumMatchesNode !== false) }
    }
    runs[which].push({ round: r, load, cell })
    console.error(`round ${r} ${which} load ${load.toFixed(1)} ${JSON.stringify(cell)}`)
  }
}
install('new')
const med = (xs) => { const s = [...xs].sort((a, b) => a - b); return s.length % 2 ? s[(s.length - 1) / 2] : (s[s.length / 2 - 1] + s[s.length / 2]) / 2 }
const summary = {}
for (const w of ['marked', 'minisearch', 'fflate']) {
  // The Rust arms of one run (rust, rust-a, rust-b) are three timings of the same artifact; their
  // median is that run's figure.
  const figure = (run) => med([run.cell[w].rust, run.cell[w].a, run.cell[w].b])
  const paired = runs.new.map((run, i) => figure(run) / figure(runs.base[i]))
  const aa = [...runs.base, ...runs.new].map((run) => run.cell[w].a / run.cell[w].b)
  summary[w] = {
    baseP50Ms: +med(runs.base.map(figure)).toFixed(3), newP50Ms: +med(runs.new.map(figure)).toFixed(3),
    pairedNewOverBase: +med(paired).toFixed(3), pairedRange: [+Math.min(...paired).toFixed(3), +Math.max(...paired).toFixed(3)],
    aaMedian: +med(aa).toFixed(3), aaRange: [+Math.min(...aa).toFixed(3), +Math.max(...aa).toFixed(3)],
    checksumsOk: [...runs.base, ...runs.new].every((run) => run.cell[w].checksumOk),
  }
}
writeFileSync(`${T}/summary.json`, JSON.stringify({ rounds, runs, summary }, null, 2))
console.log(JSON.stringify(summary, null, 2))
