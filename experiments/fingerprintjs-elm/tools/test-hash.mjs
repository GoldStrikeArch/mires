#!/usr/bin/env node

import { readFile, mkdir, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { wrapElm } from './wrap-elm.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const elm = '/usr/local/bin/elm'
const out = resolve(root, 'build/elm.js')

await mkdir(resolve(root, 'build'), { recursive: true })
const made = spawnSync(elm, ['make', 'src/Fingerprint.elm', '--optimize', '--output=build/elm.js'], {
  cwd: root,
  encoding: 'utf8',
  stdio: ['ignore', 'pipe', 'pipe'],
})
if (made.status !== 0) {
  process.stderr.write(made.stdout || '')
  process.stderr.write(made.stderr || '')
  process.exit(made.status ?? 1)
}

const source = await readFile(out, 'utf8')
const wrapped = wrapElm(source)
const wrappedPath = resolve(root, 'build/elm.mjs')
await writeFile(wrappedPath, wrapped)

const { default: Elm } = await import(wrappedPath)
const app = Elm.Fingerprint.init()
let last
app.ports.response.subscribe((value) => {
  last = value
})

function call(payload) {
  last = undefined
  app.ports.request.send(payload)
  if (last === undefined) throw new Error('Elm port did not respond synchronously')
  if (last.error) throw new Error(last.error)
  return last.result
}

function units(input) {
  const out = new Array(input.length)
  for (let i = 0; i < input.length; i++) out[i] = input.charCodeAt(i)
  return out
}

function hash(input, seed = 0) {
  return call({ op: 'hash', units: units(input), seed })
}

const fixtures = JSON.parse(
  await readFile(resolve(root, '../fingerprintjs-compare/fixtures/hash-vectors.json'), 'utf8'),
)

const longText =
  'Sociosqu libero in, gravida curae lobortis imperdiet. Elit odio consequat magna, molestie adipiscing phasellus, ' +
  'scelerisque urna integer id ad quam sit etiam. Etiam fringilla sodales, rhoncus imperdiet viverra ultricies ' +
  'congue. Porttitor etiam, nam a imperdiet. Leo himenaeos, mattis vitae senectus. Imperdiet convallis, congue sem ' +
  'tortor nulla. Lacus non conubia, hendrerit quisque, class lorem primis elit curabitur morbi felis. Hendrerit ' +
  'etiam tempus, accumsan urna nec torquent laoreet. Nisl tristique nibh, dictum ligula, litora ' +
  'risus vulputate porta volutpat aliquam. Aenean morbi convallis porttitor, nunc viverra egestas, tincidunt leo nam ' +
  'cubilia enim ligula rhoncus luctus. Porttitor fringilla suscipit, cras enim, aenean pulvinar morbi litora massa. ' +
  'Mollis magna facilisis orci, aptent justo inceptos, aliquet felis commodo turpis arcu donec class eros. Quisque ' +
  'fames per, ante ligula, curae pulvinar potenti aenean placerat. Facilisis semper fringilla scelerisque, fusce ' +
  'ante vulputate himenaeos malesuada. Ut platea, maecenas cursus ullamcorper. Nisl etiam nunc, diam tristique, ' +
  'dapibus metus malesuada scelerisque nam.\n' +
  'Augue sodales pulvinar, vehicula condimentum, suscipit neque netus non potenti libero. Scelerisque egestas justo, ' +
  'in tincidunt, tempor netus lacus habitasse cursus. Molestie habitant porta, hendrerit aptent, urna etiam class ' +
  'integer diam curabitur. Nec feugiat iaculis, lectus ac, mi sodales arcu purus nulla semper. Dictum eu nulla, ' +
  'tempor nullam, purus duis odio taciti id.\n' +
  'Eros lorem, nulla leo at. Ultricies viverra aliquet, sollicitudin condimentum, pulvinar elementum ante non aptent ' +
  'molestie. Varius suscipit vivamus, cras netus auctor aenean. Potenti diam augue cursus, ligula ipsum purus, mi ' +
  'vestibulum elementum auctor lorem enim tellus. Interdum amet, viverra dictumst odio dapibus.' +
  'Nam scelerisque litora himenaeos, maecenas sed enim, per bibendum erat gravida tincidunt porttitor egestas ' +
  'lacinia. Libero turpis rhoncus nulla, aliquam etiam ac, per eleifend nisi luctus vitae nunc. Etiam accumsan, ' +
  'senectus felis ad. Curabitur dui lobortis, mauris metus, est nec proin eleifend gravida magna laoreet. Porta ' +
  'vitae, himenaeos turpis integer varius. Vehicula fusce enim nostra, aliquam nisi nec ultricies aliquet.'

const extra = [
  { input: longText, hash: '211a6f425b82e115fb52ccdc51edb290' },
  {
    input: 'Hello, world, hi'
      .split('')
      .map((char) => String.fromCharCode(char.charCodeAt(0) + 256))
      .join(''),
    hash: '460892d2cab76edff07f62a97e106f6b',
  },
]

const problems = []
function check(label, actual, expected) {
  if (actual !== expected) {
    problems.push(`${label}: expected ${expected}, got ${actual}`)
  } else {
    console.log(`ok  ${label} = ${actual}`)
  }
}

for (const vector of fixtures.vectors) {
  check(`hash(${JSON.stringify(vector.input)})`, hash(vector.input), vector.hash)
}
for (const vector of extra) {
  check(`hash(extra)`, hash(vector.input), vector.hash)
}

check('hash("")', hash(''), '00000000000000000000000000000000')
check('hash(hello)', hash('hello'), 'cbd8a7b341bd9b025b1e906a48ae1d19')
check('hash(unicode emoji)', hash('Unicode: \ud83d\ude00'), 'bd1b3ef20279acc227489cd9c83a9e59')
check(
  'hash(lone surrogate, seed 17)',
  hash('\ud800 lone surrogate', 17),
  '990b62d74c7838b089ea31ef80987201',
)

const canonical = call({
  op: 'canonical',
  items: [
    { key: 'z', value: 'undefined' },
    { key: 'b|c:d\\e', value: '[1,true]' },
    { key: 'a', value: 'error' },
  ],
})
check('canonical', canonical, 'a:error|b\\|c\\:d\\\\e:[1,true]|z:undefined')

const order = call({ op: 'sourceOrder' })
if (order.length !== 42) problems.push(`source order length ${order.length}`)
if (order[0] !== 'userAgentData' || order[41] !== 'webGlExtensions') {
  problems.push(`source order ends: ${order[0]} … ${order[order.length - 1]}`)
} else {
  console.log('ok  source order (42)')
}

if (problems.length) {
  for (const problem of problems) console.error(problem)
  process.exit(1)
}
console.log('hash / canonical / source-order vectors passed')
