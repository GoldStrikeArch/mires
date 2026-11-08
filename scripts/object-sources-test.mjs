import assert from 'node:assert/strict'
import { readFile, writeFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
const [program, library] = process.argv.slice(2)
const map = JSON.parse(await readFile(program + '.map'))
const source = await readFile(library, 'utf8')
const index = map.sources.indexOf('library.rs')
assert.ok(index >= 0, 'external source must remain mapped after its source file is removed')
assert.equal(map.sourcesContent[index], source, 'map embeds the exact compiled dependency text')
assert.ok(map.sourcesContent.every(value => typeof value === 'string'), 'all mapped dependency and sysroot contents are embedded')
const { calculate } = await import(pathToFileURL(program))
for (const value of [0, 1, 17, 2147483648, 4294967295]) assert.equal(calculate(value), ((value + 7) + value) >>> 0)
await writeFile(program + '.test.json', JSON.stringify({ passed: true, checks: 8, sources: map.sources.length }) + '\n')
console.log('object sources: 8 checks passed')
