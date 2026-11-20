import assert from 'node:assert/strict'
import {resolve} from 'node:path'
import {readFileSync} from 'node:fs'
import {variant} from './bench-variant.mjs'
import {gunzipSync} from 'node:zlib'
import {fileURLToPath} from 'node:url'
import {startServer,openBrowser,runInPage} from '../../tools/lib/browser.mjs'
import {moduleGraph,repositoryRoot} from '../../tools/lib/registry.mjs'

const [browser='chrome', ...requested] = process.argv.slice(2)
const languages = requested.length ? requested : ['reference','rust','rescript','melange','jsoo','scalajs','dart','kotlin','elm','gopherjs']
const file = path=>fileURLToPath(new URL(path,import.meta.url))
const bench=JSON.parse(readFileSync(file('corpus.json'))).cases.find(c=>c.id==='bench-offline').input
const seeds=[1,18,35,2**26+1,2**32+1]
const variants=seeds.map(k=>variant(bench,k))
assert.equal(new Set(variants.map(input=>JSON.stringify(input.chart))).size,seeds.length)
assert.equal(new Set(variants.map(input=>input.records[0].id)).size,seeds.length)
for(const input of variants) {
  assert.equal(input.records.length,bench.records.length)
  assert.equal(input.chart.frames[0].data[0].length,bench.chart.frames[0].data[0].length)
}
const oracle=file('reference.mjs'), driver=file('capture.mjs')
const graph=await moduleGraph(oracle)
assert.deepEqual(graph.errors,[])
const targets=languages.map(id=>id==='reference'?oracle:file('../../ports/explorer/'+id+'/dist/index.min.js'))
const server=await startServer([driver,...targets,...graph.modules.map(m=>resolve(repositoryRoot,m.path))])
const inputs = Array.from({length:6},(_,i)=>{
  const title='target'+i
  const body='# '+title+'\n\nCafé **'+i+'**'
  const selected = i%2 === 0 ? {id:'a',title,body} : {body,id:'a',title}
  const filled={size:[80,30],xRange:[0,4],yRange:[-4,8],data:[[0,1,2,3,4],[i-1,null,0,i,4]],probes:[[1,0,40,15]]}
  const empty={...filled,size:[80.5,30.75],data:[[],[]]}
  const frames=[[filled],[empty],[filled,empty],[empty,filled],[filled,{...empty,probes:[]}],[filled,empty,filled]][i]
  return {
    records:[{id:'b',title:'unused',body:'second document'}, {id:'a',title:'old',body:'this is replaced'},
      selected], query:title,
    chart:{spanGaps:i%2===0,frames}
  }
})
let session
try {
  session=await openBrowser(browser)
  if(session.error) throw Error(session.error)
  for (const [index,language] of languages.entries()) {
    const load=index+1
    const url=path=>server.url(path,'?load='+load)
    const result=await runInPage(session.driver,server.origin,load,url(driver),'capture',{
      moduleUrl:url(targets[index]),oracleUrl:url(oracle),inputsJSON:JSON.stringify(inputs)
    })
    if(result.error) throw Error(language+' '+result.error.message)
    assert.deepEqual(result.cspViolations,[])
    assert.deepEqual(result.after,result.before,language+' leaked a database or DOM node')
    assert.deepEqual(result.results,result.expected,language)
    for (const [i,[ids,html,gzip,byteCount,chart]] of result.results.entries()) {
      for (const [index,frame] of inputs[i].chart.frames.entries()) {
        if (frame.data[0].length) continue
        const [width,height,paths,probes]=chart[index]
        assert.deepEqual([width,height],[81,31])
        assert.deepEqual(paths,[[[],inputs[i].chart.spanGaps?null:[]]])
        assert.deepEqual(probes.map(p=>p.slice(4)),frame.probes.map(()=>[-1,-1]))
      }
      assert.deepEqual(ids,['a'])
      assert.equal(html,'<h1>target'+i+'</h1>\n<p>Café <strong>'+i+'</strong></p>\n')
      const raw=gunzipSync(Uint8Array.from(gzip))
      assert.equal(raw.length,byteCount)
      const surviving=[inputs[i].records[2],inputs[i].records[0]]
      assert.equal(raw.toString('utf8'),surviving.map(doc=>JSON.stringify(doc)).join('\n'),language+' archive bytes/key order')
      const docs=raw.toString('utf8').split('\n').map(line=>JSON.parse(line))
      assert.deepEqual(docs,[inputs[i].records[2],inputs[i].records[0]],language+' archive order/content')
    }
    console.log(browser+' '+language+' passed six overlapping calls, archive decoding and cleanup')
  }
} finally {await session?.driver?.quit();await server.close()}
