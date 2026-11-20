import { writeFileSync } from 'node:fs'

const cases = []
const edges = [[0,0],[1,0],[-1,-1],[0,-2147483648],[-1,2147483647],[1,-2147483648],[0,-1],[-1,0],[0,1],[1,1],[0,2147483647],[-2,-1],[0x40000000,0],[0,0x40000000]]
const ops = ['fromBits','toString','add','subtract','multiply','divide','modulo','and','or','xor','shiftLeft','shiftRight','shiftRightUnsigned','rotl','clz','ctz','compare']
const cmd = (op, args={}) => ({op,a:[1,0],b:[3,0],u:false,v:false,n:0,r:10,s:'',...args})
const add = (id,description,lines) => cases.push({id,description,input:{cases:lines}})
for (const op of ops) {
  for (const u of [false,true]) {
    const lines=[]
    if (['add','subtract','multiply','divide','modulo','and','or','xor','compare'].includes(op)) {
      for(const a of edges) for(const b of edges) for(const v of [false,true]) lines.push(cmd(op,{a,b,u,v,r:16}))
    } else if (['shiftLeft','shiftRight','shiftRightUnsigned','rotl'].includes(op)) {
      for(const a of edges) for(const n of [-65,-1,0,1,15,31,32,33,63,64,65,130]) lines.push(cmd(op,{a,n,u,r:16}))
    } else {
      for(const a of edges) for(const r of [0,2,3,8,10,16,32,36]) lines.push(cmd(op,{a,u,r}))
    }
    add(op+'-'+(u?'unsigned':'signed'),op+' over edge bit patterns',lines)
  }
}
add('wide-shift-counts','Shift and rotation counts wrap modulo 64 across signed word boundaries',
  ['shiftLeft','shiftRight','shiftRightUnsigned','rotl'].flatMap(op =>
    [-1,1].flatMap(sign => [2**31-1,2**31,2**31+1,2**32-1,2**32,2**32+1,Number.MAX_SAFE_INTEGER]
      .map(width => cmd(op,{a:[1234,-14],n:sign*width,u:true,r:16})))))
for (let radix=2;radix<=36;radix++){
  const lines=[]
  for (const x of [0n,1n,2n**32n-1n,2n**63n-1n,2n**63n,2n**64n-1n,2n**80n+123n]) {
    for (const sign of ['', '-']) for (const u of [false,true]) {
      lines.push(cmd('fromString',{s:sign+x.toString(radix),u,n:radix,r:radix}))
    }
  }
  add('parse-radix-'+radix,'8-digit parsing chunks and 6-digit formatting groups',lines)
}
add('parsing-errors','empty input, signs, special values, parseInt prefixes and chunk boundaries',
  ['', '-', '--1', '---123','1-2','NaN','Infinity','-Infinity','+Infinity','  +42','0x101b','+0x1','1z234567888','x123456700000001','1234 5678','ZzZ','\t80','   5'].flatMap(s =>
    [0,1,10,16,36,37].map(n=>cmd('fromString',{s,n,r:16}))).concat([cmd('toString',{r:1}),cmd('toString',{r:37}),cmd('unknown')]))
add('wasm-divide-guard','the pinned default returns the original dividend when high is MIN_INT32 and divisor is -1',
  [0,1,42,-1].map(lo=>cmd('divide',{a:[lo,-2147483648],b:[-1,-1],r:16})))
let seed=0x4c4f4e47
const random=()=>{seed^=seed<<13;seed^=seed>>>17;seed^=seed<<5;return seed|0}
for(let j=0;j<8;j++){
  const lines=[]
  for(let i=0;i<100;i++){const op=ops[(random()>>>0)%ops.length];lines.push(cmd(op,{a:[random(),random()],b:[random(),random()],n:random(),u:random()<0,v:random()<0,r:2+(random()>>>0)%35}))}
  add('random-'+j,'deterministic combinations of words, signs and radices',lines)
}
for(const u of [true,false])add('bench-'+(u?'unsigned':'signed'),'100,000 multiply, division and rotate iterations with 12,500 radix conversions',
  [cmd('mix',{a:[0x12345678,-123456789],b:[0x504c414e,1234567],n:100000,u,v:u,r:u?16:10})])
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'long',cases})+'\n')
