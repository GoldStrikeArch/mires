import assert from 'node:assert/strict'
import {variant} from './bench-variant.mjs'
import {readFileSync,writeFileSync} from 'node:fs'
import {fileURLToPath} from 'node:url'
import {openBrowser,runInPage,startServer} from '../../tools/lib/browser.mjs'
const [browser='chrome', language='reference', action] = process.argv.slice(2)
const file = p => fileURLToPath(new URL(p,import.meta.url))
const target = file(language==='reference'?'reference.mjs':'../../ports/workbox/'+language+'/dist/index.min.js')
const driver = file('capture.mjs')
const cases = JSON.parse(readFileSync(file('corpus.json'))).cases
const bench = cases.find(c=>c.id==='bench-cache').input
const seeds = [1, 2**32+1, 2**52+1, Number.MAX_SAFE_INTEGER]
const variants = seeds.map(k=>variant(bench,k))
for (const input of variants) assert.equal(input.steps.length,bench.steps.length)
assert.ok(bench.steps.some(step=>step.path?.startsWith('first/')))
for (const i of bench.steps.keys()) {
  if (!bench.steps[i].path?.startsWith('first/')) continue
  const targets = variants.map(input=>input.steps[i].path)
  assert.equal(new Set(targets).size,seeds.length)
  assert.equal(new Set(targets.map(target=>target.length)).size,1)
}
const server = await startServer([driver,target,file('alpha.mjs'),file('beta.mjs'),file('../../node_modules/workbox-precaching/matrix-entry.mjs')])
let session
try {
  session=await openBrowser(browser)
  if(session.error) throw Error(session.error)
  const r=await runInPage(session.driver,server.origin,1,server.url(driver,'?load=1'),'capture',{moduleUrl:server.url(target,'?load=1'),cases})
  if(r.error) throw Error(r.error.message)
  assert.deepEqual(r.before,r.after)
  assert.deepEqual(r.cspViolations,[])
  for(const c of cases){
    if (c.id==='conflict') { assert.ok(r.results[c.id]?.error, 'conflicting revisions should reject');r.results[c.id]={error:'conflict'} }
    else assert.ok(Array.isArray(r.results[c.id]), c.id+' '+JSON.stringify(r.results[c.id]))
  }
  if(action==='--capture'){
    assert.equal(language,'reference')
    writeFileSync(file('expected.json'),JSON.stringify(r.results,null,2)+'\n')
  } else assert.deepEqual(r.results,JSON.parse(readFileSync(file('expected.json'))))
  console.log(browser+' '+language+' matched '+cases.length+' scenarios and restored cache names')
} finally { await session?.driver?.quit();await server.close() }
