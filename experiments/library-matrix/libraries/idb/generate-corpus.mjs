import {writeFileSync} from 'node:fs'
const cases=[]
const row=(id,group,value)=>({id,group,value})
const a=row(3,'b','three'), b=row(1,'a','one'), c=row(2,'a','two'), d=row(-1,'a','negative')
const basic={batches:[{rows:[a,b,c,d]}],keys:[1,3,-1,2,0,99],groups:['a','b','missing'],index:false,reverse:false,step:1}
function add(id,changes) {cases.push({id,input:{...basic,...changes}})}
add('empty',{batches:[],keys:[],groups:[]})
add('empty-transaction',{batches:[{rows:[]},{rows:[],abort:true}]})
add('store-order',{})
add('index-order',{index:true})
add('reverse-store',{reverse:true,step:2})
add('reverse-index',{index:true,reverse:true,step:1})
add('index-advance',{index:true,step:3})
add('index-equality',{index:true,only:'a'})
add('index-no-match',{index:true,only:'absent'})
add('store-equality',{only:2})
add('overwrite',{batches:[{rows:[a,b,c]},{rows:[row(1,'b','updated'),row(1,'c','last')]}],groups:['a','b','c'],index:true})
add('explicit-abort',{batches:[{rows:[a,b]},{rows:[row(1,'bad','overwritten'),c],abort:true},{rows:[d]}],index:true})
add('constraint-abort',{batches:[{rows:[b]},{rows:[a,b,c],add:true},{rows:[d],add:true}]})
add('unicode-empty',{batches:[{rows:[row(0,'',''),row(1,'é','雪'),row(2,'é','🙂'),row(3,'é','duplicate')]}],index:true,groups:['','é','é','x'],keys:[0,3]})
const records=Array.from({length:64},(_,i)=>row((i*29)%64,'group-'+i%7,'value-'+i))
add('bench-transactions',{batches:[{rows:records.slice(0,32)},{rows:records.slice(32)}],keys:records.map(r=>r.id),groups:['group-0','group-3','group-6'],index:true,step:3})
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'idb',cases})+'\n')
