import assert from 'node:assert/strict'
import {variant} from './bench-variant.mjs'
import {readFileSync,writeFileSync} from 'node:fs'
import {fileURLToPath} from 'node:url'
import {openBrowser,runInPage,startServer} from '../../tools/lib/browser.mjs'
const [browser='chrome',language='reference',action] = process.argv.slice(2)
const file = s => fileURLToPath(new URL(s,import.meta.url))
const driver=file('capture.mjs'), oracle=file('reference.mjs')
const target=language==='reference'?oracle:file('../../ports/uplot/'+language+'/dist/index.min.js')
const corpus=JSON.parse(readFileSync(file('corpus.json'))).cases
const fixture=corpus.find(entry=>entry.id==='bench-lines').input
const baseline=structuredClone(fixture)
const seeds=[...Array.from({length:64},(_,i)=>i+1),1_000_001,2**26+1,2**32+1,Number.MAX_SAFE_INTEGER-1,Number.MAX_SAFE_INTEGER]
const origins=new Set()
for(const k of seeds) {
  const [frame]=variant(fixture,k).frames
  const dx=frame.xRange[0]-baseline.frames[0].xRange[0]
  const dy=baseline.frames[0].yRange[0]-frame.yRange[0]
  origins.add(JSON.stringify([dx,dy]))
  assert.deepEqual(frame.data[0].map(x=>x-dx),baseline.frames[0].data[0])
  for (const [index,values] of frame.data.slice(1).entries())
    assert.deepEqual(values.map(y=>y===null?null:y+dy),baseline.frames[0].data[index+1])
}
assert.equal(origins.size,seeds.length)
assert.deepEqual(fixture,baseline)
const scanned=Array.from({length:100},(_,i)=>{
  const width=10+i%32, height=22+i%44, n=20+(i*17)%280
  const xs=Array.from({length:n},(_,j)=>Math.floor(j*0.83))
  const ys=xs.map((_,j)=>(j*111+i)%23<3?null:(i+j*17)%39-17)
  return {id:'seed-'+i,input:{spanGaps:i%5===0,frames:[{size:[width,height],xRange:[0,xs.at(-1)],yRange:[-22,22],data:[xs,ys],probes:[[i/7,-5,width/3,height/7]]}]}}
})
const server=await startServer([driver,oracle,target,file('../../node_modules/uplot/dist/uPlot.esm.js')])
let session
try {
  session=await openBrowser(browser)
  if (session.error) throw Error(session.error)
  const sample=async(cases,seeded)=> {
    const res=await runInPage(session.driver,server.origin,1,server.url(driver,'?load=1'),'capture',{moduleUrl:server.url(target,'?load=1'),oracleUrl:server.url(oracle,'?load=1'),cases,seeded})
    if(res.error) throw Error(res.error.message)
    const {cspViolations,...output}=res
    assert.deepEqual(cspViolations,[])
    return output
  }
  const results=await sample(corpus,false)
  if(action==='--capture') {
    assert.equal(language,'reference')
    writeFileSync(file('expected.json'),JSON.stringify(results)+'\n')
  } else {
    assert.deepEqual(results,JSON.parse(readFileSync(file('expected.json'))))
    await sample(scanned,true)
  }
  console.log(browser+' '+language+' matched '+corpus.length+' chart snapshots'+(action==='--capture'?'':' and 100 generated charts'))
} finally {await session?.driver?.quit(); await server.close()}
