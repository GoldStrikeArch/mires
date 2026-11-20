import { writeFileSync } from 'node:fs'
import { run, units } from './reference.mjs'

const cases = []
function add(id, input) { cases.push({ id, input }) }
const samples = {
  empty: [], hello: units('Hello world!'), repeating: units('aaaaabaaaaacaaaaadaaaaaeaaaaa'),
  mixed: units('日本語 🧪 Привет 🌻 مرحبا\r\n'), surrogates: [0xdfff, 32, 0xd800, 0xd800, 0xdfff, 0],
  literals: [0, 1, 2, 254, 255, 256, 257, 65534, 65535], words: units('TOBEORNOTTOBEORTOBEORNOT'),
  keys: units('__proto__ constructor toString __proto__'.repeat(100)),
  printable: Array.from({length: 768}, (_, i) => (i * 563 + 32) & 65535),
}
for (const format of ['raw', 'base64', 'utf16', 'uri', 'bytes']) {
  for (const [name, data] of Object.entries(samples)) {
    const input = { op: 'compress', format, data }
    add(name + '-' + format, input)
    add('decode-' + name + '-' + format, { op: 'decompress', format, data: run(input) })
  }
  for (const op of ['compress', 'decompress']) add(op + '-null-' + format, { op, format, data: null })
  for (const [id, data] of [['empty', []], ['zero', [0, 0]], ['cut', run({op:'compress',format,data:samples.mixed}).slice(0, 2)]]) {
    add('invalid-' + id + '-' + format, { op: 'decompress', format, data })
  }
}
add('odd-bytes', { op: 'decompress', format: 'bytes', data: [64] })
add('uri-space', { op: 'decompress', format: 'uri', data: run({op:'compress',format:'uri',data:samples.printable}).map(n => n === 43 ? 32 : n) })
const record = units(JSON.stringify({city:'京都', greeting:'🌻 Привет', value:1024, tags:['hello','\ud800','β']}) + '\n')
add('bench-base64', { op: 'roundtrip', format: 'base64', data: record, repeat: Math.ceil(100000 / record.length) })
writeFileSync(new URL('corpus.json', import.meta.url), JSON.stringify({schema:1,library:'lz-string',cases})+'\n')
