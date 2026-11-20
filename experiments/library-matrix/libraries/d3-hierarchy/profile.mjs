// Profile the same shapes used in the matrix and count emitted heap operations on one layout.
import { readFileSync } from 'node:fs'
import { Session } from 'node:inspector/promises'
import { variant } from './bench-variant.mjs'

const operation = process.argv[2]
if (!['pack', 'treemap'].includes(operation)) throw Error('Choose pack or treemap')
const corpus = JSON.parse(readFileSync(new URL('corpus.json', import.meta.url)))
const input = corpus.cases.find(test => test.id === 'bench-' + operation).input
const variants = Array.from({ length: 8 }, (_, index) => variant(input, index))
const artifact = new URL('../../ports/d3-hierarchy/rust/dist/index.js', import.meta.url)
const { run } = await import(artifact)
for (const sample of variants) run(sample)

const session = new Session()
session.connect()
let profile
try {
  await session.post('Profiler.enable')
  await session.post('Profiler.setSamplingInterval', { interval: 250 })
  await session.post('Profiler.start')
  for (let i = 0; i < 65; i++) run(variants[i % variants.length])
  const result = await session.post('Profiler.stop')
  profile = result.profile
} finally {
  session.disconnect()
}

const names = new Map(profile.nodes.map(node => [node.id, node.callFrame.functionName || '(anonymous)']))
const micros = new Map()
for (const [index, id] of profile.samples.entries()) {
  const name = names.get(id)
  micros.set(name, (micros.get(name) ?? 0) + profile.timeDeltas[index])
}
const total = [...micros.values()].reduce((a, b) => a + b, 0)
console.log(operation, process.version)
console.table([...micros].sort((a, b) => b[1] - a[1]).slice(0, 12)
  .map(([name, time]) => ({ name, 'CPU self (%)': (100 * time / total).toFixed(1) })))

// Count calls in a separate in-memory copy so instrumentation does not affect the CPU samples.
let source = readFileSync(artifact, 'utf8')
const declarations = [
  ['$rt__register', 'buf, bytes, zeroed', 'bytes'],
  ['$rt__blk_get', 'buf', '"all"'],
  ['$rt__put', 'buf, off, value, heap', 'Object.keys(value ?? {}).join(",")'],
  ['$rt_overwrite', 'target, value', 'Object.keys(value ?? {}).join(",")'],
]
for (const [name, args, key] of declarations) {
  const declaration = 'let ' + name + ' = (' + args + ') => {'
  if (!source.includes(declaration)) throw Error('Generated runtime signature changed for ' + name)
  source = source.replace(declaration, () => declaration + '\ncount(' + JSON.stringify(name) + ',' + key + ');')
}
const counter = `
export const counts = Object.create(null);
function count(name, key) {
  const byShape = counts[name] ??= Object.create(null);
  byShape[key] = (byShape[key] ?? 0) + 1;
}
`
const instrumented = await import('data:text/javascript;base64,' + Buffer.from(counter + source).toString('base64'))
instrumented.run(variants[0])
for (const [name, counts] of Object.entries(instrumented.counts)) {
  const calls = Object.values(counts).reduce((a, b) => a + b, 0)
  console.log(name, calls.toLocaleString('en-US'), 'calls', counts)
}
