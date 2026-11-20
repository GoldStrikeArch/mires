import { readFileSync, writeFileSync } from 'node:fs'
const corpus = []
const add = (id, description, cases, repeat=1) => corpus.push({id,note:description,input:{cases,repeat}})
const decimal = (len,seed) => {
  let s = String(1 + seed % 9)
  for (let i=1;i<len;i++) { seed = (Math.imul(seed,1664525)+1013904223) >>> 0; s += seed % 10 }
  return s
}
add('construct','Decimal grammar, number conversion, notation and invalid inputs',
  ['0','-0','-.0','.1','1.','00.0100','1e-7','1e-6','1E+20','1e+21','-123e-500',0,1e-7,1e21,-0.314159,' 1','1 ','+1','.','', '1e','1e+','NaN','Infinity','--1','1.2.3','12a'].map(x=>({x,op:'toString'})))
for (const op of ['plus','minus','times','div','cmp']) {
  let rows=[]
  for (const x of ['0','-0','-999','999','-0.002','10.00001','1e-9']) for (const y of ['-0','-999','0.002','3','1e8']) rows.push({x,y,op})
  for (const n of [1,7,20,50,100,251,500]) {
    let x=decimal(n,n+10), y=decimal(n,n+21)
    for(const rm of [0,1,2,3]) rows.push({op,x:x+'e-'+Math.floor(n/2),y:'-'+y+'e-'+Math.floor(n/2),rm,dp:38})
  }
  add(op,'Signs, carry, borrow and seeded digit arrays',rows)
}
for(const op of ['round','toPrecision','toExponential','toFixed']) {
  let rows=[]
  for(const x of ['0','-0','-0.0005','999.95','12.5','13.5','0.05501','100000000000000000000']) for(const n of [undefined,-2,0,1,2,6]) for(const mode of [0,1,2,3]) rows.push({op,x,n,mode})
  for(const n of [0.5,-1000001,-2000002,1000001]) rows.push({op,x:'1',n})
  rows.push({op,x:'0',n:1,mode:4})
  add(op,'Precision, padding, signs and rounding modes',rows)
}
add('pow','Positive, negative and invalid powers',[-13,-2,-1,0,1,2,7,12,0.5,1000001].flatMap(n=>['0','-0','-1.25','3.123'].map(x=>({op:'pow',x,n}))))
add('sqrt','Newton iteration, extreme scales, perfect squares and ties',['-1','-0','0','0.25','2','9','2.25','6.25','6.25000001','1e400','1e-401','1.23456e-30','9999800001',decimal(120,13)].flatMap(x=>[0,1,2,3].flatMap(rm=>[0,1,20].filter(dp=>(rm!==3 || !x.includes('e-')) && (x!=='1.23456e-30' || dp===20)).map(dp=>({x,op:'sqrt',dp,rm})))))
add('errors','Invalid operand, decimal places, mode and division by zero',[
  {op:'times',x:'1',y:''},{op:'div',x:'0',y:'0'},{op:'div',x:'1',y:'2',dp:-1},{op:'div',x:'1',y:'7',rm:5},{op:'round',x:'1',rm:-1}
])
const wide = JSON.parse(readFileSync(new URL('exponent-vectors.json',import.meta.url)))
add('wide-exponents','Parsing, coefficient alignment, arithmetic and rounding across 32-bit exponent bounds',wide.map(v=>v.case))
const upstream = JSON.parse(readFileSync(new URL('upstream-vectors.json',import.meta.url)))
add('upstream','Selected assertions from the upstream big.js v7.0.1 method tests',upstream.map(v=>v.case))
const fifty=decimal(50,37), other=decimal(50,81)
add('bench-mixed','10,000 mixed operations on 50-digit coefficients',
  ['plus','minus','times','div','cmp','toString','round','toFixed','toExponential','toPrecision'].map(op=>({op,x:fifty,y:other,n:9})),1000)
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'big-js',cases:corpus})+'\n')
