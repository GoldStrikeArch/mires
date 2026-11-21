import assert from 'node:assert/strict'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { openBrowser, runInPage, startServer } from '../../tools/lib/browser.mjs'
import { variant } from './bench-variant.mjs'

const [browser='chrome', language='reference', action] = process.argv.slice(2)
const file = relative => fileURLToPath(new URL(relative,import.meta.url))
const corpus = JSON.parse(readFileSync(file('./corpus.json'))).cases
// Two bench variants, the second past 2^32, are checked like corpus cases.
const bench = corpus.find(c=>c.id==='bench-transactions').input
const cases = [...corpus, ...[7,2**32+1].map(k=>({id:'bench-transactions~'+k,input:variant(bench,k)}))]
const target = file(language==='reference'?'./reference.mjs':`../../ports/idb/${language}/dist/index.min.js`)
const driverModule = file('./capture.mjs')
const fixture = file('./expected.json')
if(action==='--capture' && language!=='reference') throw Error('Capture requires the pinned package reference')
const server = await startServer([driverModule,target,file('../../node_modules/idb/build/index.js')])
let session
try {
  session = await openBrowser(browser)
  if(session.error) throw Error(session.error)
  const result = await runInPage(session.driver,server.origin,1,server.url(driverModule,'?load=1'),'capture',
    {moduleUrl:server.url(target,'?load=1'),casesJSON:JSON.stringify(cases)})
  if(result.error) throw Error(result.error.message)
  assert.deepEqual(result.cspViolations,[])
  assert.deepEqual(result.after,result.before,'database cleanup')
  const outputs = Object.fromEntries(cases.map((c,i)=>[c.id,JSON.parse(result.pairs[i])]))
  // One compact line per case: the deep-value case would make an indented fixture quadratic.
  if(action==='--capture') writeFileSync(fixture,'{\n'+Object.entries(outputs).map(([id,output])=>JSON.stringify(id)+': '+JSON.stringify(output)).join(',\n')+'\n}\n')
  else assert.deepEqual(outputs,JSON.parse(readFileSync(fixture)))
  console.log(`${browser}: ${cases.length} concurrent pairs (${corpus.length} cases, 2 bench variants) matched, all database names restored`)
} finally {
  await session?.driver?.quit()
  await server.close()
}
