// Page driver for verify.mjs, loaded into the page realm by runInPage. It reads the corpus, runs
// every case and the bench variants through one artifact, and returns each output as JSON text
// together with the scenario caches each call left behind (the inventory before and after a call
// must agree).
import {variant} from './bench-variant.mjs'

export async function capture({moduleUrl, corpusUrl, seeds}) {
  const {run} = await import(moduleUrl)
  const {cases} = JSON.parse(await (await fetch(corpusUrl)).text())
  const bench = cases.find(c => c.id === 'bench-cache').input
  const all = [...cases, ...seeds.map((k, i) => ({id: 'bench-variant-' + i, input: variant(bench, k)}))]
  const results = {}
  const leaks = []
  for (const entry of all) {
    const before = (await caches.keys()).sort()
    try {
      results[entry.id] = JSON.stringify(await run(structuredClone(entry.input)))
    } catch (error) {
      results[entry.id] = JSON.stringify({threw: String(error?.name), message: String(error?.message ?? error)})
    }
    const after = (await caches.keys()).sort()
    if (JSON.stringify(before) !== JSON.stringify(after)) leaks.push({id: entry.id, before, after})
  }
  return {results, leaks, inputs: Object.fromEntries(all.map(entry => [entry.id, JSON.stringify(entry.input)]))}
}
