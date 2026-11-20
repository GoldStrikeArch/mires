import { readFileSync, writeFileSync } from 'node:fs'
import Big from '../../node_modules/big.js/big.mjs'
const corpus = []
const add = (id, description, cases, repeat=1) => corpus.push({id,note:description,input:{cases,repeat}})
const decimal = (len,seed) => {
  let s = String(1 + seed % 9)
  for (let i=1;i<len;i++) { seed = (Math.imul(seed,1664525)+1013904223) >>> 0; s += seed % 10 }
  return s
}
// Upstream P.sqrt step for step (estimate, DP + 4, the joined-prefix stop test), with an iteration
// cap: a square root is in the domain only if this loop stops (CONTRACT.md). Callers keep exponents
// moderate, because a single division at DP + 4 places costs O(x.e) digits.
const sqrtTerminates = ({x:n, dp=20, rm=1}, cap=200) => {
  const D = Big(); D.DP = dp + 4; D.RM = rm
  let x, r
  try { x = new D(n) } catch { return true }
  if (!x.c[0] || x.s < 0) return true
  let s = Math.sqrt(+x.toExponential()), e = x.e
  if (s === 0 || s === 1/0) {
    let c = x.c.join('')
    if (!(c.length + e & 1)) c += '0'
    s = Math.sqrt(c)
    e = ((e + 1) / 2 | 0) - (e < 0 || e & 1)
    r = new D((s == 1/0 ? '5e' : (s = s.toExponential()).slice(0, s.indexOf('e') + 1)) + e)
  } else {
    try { r = new D(s + '') } catch { return true }
  }
  const k = r.e + D.DP
  for (let i=0;i<cap;i++) {
    const t = r
    try { r = new D('0.5').times(t.plus(x.div(t))) } catch { return true }
    if (t.c.slice(0,k).join('') === r.c.slice(0,k).join('')) return true
  }
  return false
}
const lcg = seed => () => (seed = (Math.imul(seed,1103515245)+12345) >>> 0) / 2**32
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
add('sqrt','Newton iteration, extreme scales, perfect squares and ties',['-1','-0','0','0.25','2','9','2.25','6.25','6.25000001','1e400','1e-401','1.23456e-30','9999800001',decimal(120,13)].flatMap(x=>[0,1,2,3].flatMap(rm=>[0,1,20].map(dp=>({x,op:'sqrt',dp,rm})).filter(c=>sqrtTerminates(c)))))
add('errors','Invalid operand, decimal places, mode and division by zero',[
  {op:'times',x:'1',y:''},{op:'div',x:'0',y:'0'},{op:'div',x:'1',y:'2',dp:-1},{op:'div',x:'1',y:'7',rm:5},{op:'round',x:'1',rm:-1}
])
const wide = JSON.parse(readFileSync(new URL('exponent-vectors.json',import.meta.url)))
add('wide-exponents','Parsing, coefficient alignment, arithmetic and rounding across 32-bit exponent bounds',wide.map(v=>v.case))
const upstream = JSON.parse(readFileSync(new URL('upstream-vectors.json',import.meta.url)))
add('upstream','Selected assertions from the upstream big.js v7.0.1 method tests',upstream.map(v=>v.case))

// Regressions from the 2026-11-20 pre-merge review (b8), and the adversarial classes earlier reviews
// found in shared-design ports.
// sqrt sets Big.DP += 4 and lets div validate it: dp -4..999996 are valid, dp <= -5, non-integers and
// dp past int32 throw. Zero and negative operands return or throw before that check. The large valid
// dp values use exact roots, which stop after one division.
add('sqrt-decimal-places','sqrt validates DP + 4 through div, after the zero and sign checks, before the rounding mode',[
  ...[-6,-5,-4,-3,-2,-1,0.5,-4.5,3.5,2**31+1,2**32+20,null].flatMap(dp=>['2','4','1e-30','0','-0','-1','0.0001','99'].map(x=>({op:'sqrt',x,dp}))),
  ...[999996,999997,1000000].flatMap(dp=>['4','1e-30','0','-1','0.0625'].map(x=>({op:'sqrt',x,dp}))),
  ...[5,-1,1.5,4,2**32+1,null].flatMap(rm=>['2','4','0','-1'].map(x=>({op:'sqrt',x,rm}))),
  ...[[-5,5],[0.5,-1],[-1,9],[-4,2]].map(([dp,rm])=>({op:'sqrt',x:'2',dp,rm})),
])
// The tiny-root region that the generated ports short-circuited: exponents near -2dp-2 and the
// working window r.e + DP + 4, in every rounding mode, as in the review's big3.mjs. Only the
// operations on which upstream's loop stops are kept.
{
  const rnd = lcg(777), ri = (a,b) => a + Math.floor(rnd()*(b-a+1)), pick = a => a[ri(0,a.length-1)]
  const rows = []
  for (let i=0;i<900;i++) {
    const dp = pick([0,0,1,2,3,5,10,20]), rm = ri(0,3)
    let s = String(ri(1,9)); const len = pick([1,1,2,3,6,15,30]); for (let k=1;k<len;k++) s += ri(0,9)
    if (rnd() < 0.3) s = pick(['1','25','5','225','4','9','16','625','2','1'])
    const e = rnd() < 0.6 ? -2*dp - ri(-3,6) : ri(-60,60)
    const c = {op:'sqrt',x:s+'e'+e,dp,rm}
    if (sqrtTerminates(c)) rows.push(c)
  }
  add('sqrt-tiny-roots','Square roots below and around the last requested place, all rounding modes (upstream iteration)',rows)
}
// Exponents are one Number() parse of the signed digits, then ((dot + exp) - lead) - 1 in doubles.
const nines400 = '9'.repeat(400)
add('exponent-parse','Exponents past 2^53, past 1e21 and past the double range',[
  {op:'toString',x:'999999999999999999999999999999e99999999999999999999'},
  {op:'toPrecision',x:'12.34e9007199254740993',n:3},
  ...['1e9007199254740993','1e9007199254740992','3e9007199254740995','12.34e9007199254740993','0.0012e-9007199254740993',
    '1e18014398509481985','123e18014398509481983','00012.5e9007199254740991','-0.000123e-18014398509481984','1e400000000000000000000',
    '1e1000000000000000000000','1e1'+'0'.repeat(20)+'1','9'.repeat(30)+'e99999999999999999999','1e'+nines400,'-1e-'+nines400,
    '1.5e'+'9'.repeat(309),'1e0001','1e-0','-0e5','1E5','1e+00000000000000000000000000021']
    .flatMap(x=>['toString','toExponential','toPrecision','cmp'].map(op=>({op,x,y:'1',n:op==='toPrecision'?3:undefined}))),
])
// Exponents past 2^53 round as doubles: a subtraction's leading zeros decrement ye one at a time
// (a run of --ye is not one subtraction there), a carry is one ++, and rounding up is one ++.
add('exponents-past-2^53','Sums, differences, products, rounding and exact quotients at exponents past 2^53',
  ['9007199254740993','9007199254740995','9007199254740996','9007199254740998','18014398509481988','18014398509481990','36028797018963971'].flatMap(E=>[
    ...[['1.001','1'],['1.0000001','1'],['1.00001','-1'],['1','1'],['5','5'],['9.99','0.01'],['1.5','-1.49'],['2','-1.999']]
      .flatMap(([a,b])=>[{op:'minus',x:a+'e'+E,y:b+'e'+E},{op:'plus',x:a+'e'+E,y:b+'e'+E}]),
    {op:'times',x:'5e'+E,y:'2e'+E},{op:'times',x:'1.5e'+E,y:'3e-5'},{op:'times',x:'3e'+E,y:'3e'+E},
    {op:'round',x:'9.99e'+E,n:-Number(E),mode:1},{op:'toPrecision',x:'9.99e'+E,n:2},{op:'toExponential',x:'9.99e'+E,n:1},{op:'round',x:'9.99e'+E},
    {op:'div',x:'6e'+E,y:'2'},{op:'div',x:'1e-'+E,y:'7'},{op:'div',x:'1e-'+E,y:'7',rm:3},{op:'cmp',x:'1e'+E,y:'1.0e'+E},
    {op:'pow',x:'2e'+E,n:2},{op:'pow',x:'1e-'+E,n:-1}]))
// Exponents of Infinity and NaN (Infinity - Infinity) keep big.js's comparison chains: NaN fails
// every test, so stringify prints the bare coefficient and cmp falls through to the sign. Sums with
// an infinite exponent difference, toFixed of an infinite positive exponent and inexact quotients
// with an infinite quotient exponent never finish (or hit the host's array limit).
const huge = '1e'+nines400, tiny = '1e-'+nines400
add('nonfinite-exponents','Infinite and NaN exponents in formatting, comparison, products and quotients',[
  ...[huge,'-1.5e'+nines400,tiny,'-25e-'+nines400].flatMap(x=>[
    {op:'toString',x},{op:'toExponential',x,n:2},{op:'toExponential',x},{op:'toPrecision',x,n:2},{op:'toPrecision',x},{op:'round',x},
    {op:'round',x,mode:3},{op:'round',x,n:-3,mode:0},{op:'cmp',x,y:huge},{op:'cmp',x,y:'2e'+nines400},{op:'cmp',x,y:'-'+tiny},{op:'cmp',x,y:'0'},
    {op:'times',x,y:tiny},{op:'times',x,y:'12e'+nines400},{op:'times',x,y:'0'},{op:'div',x,y:'3e'+nines400},{op:'div',x,y:'7e'+nines400,dp:3},
    {op:'pow',x,n:2},{op:'pow',x,n:-1},{op:'sqrt',x}]),
  ...[tiny,'-25e-'+nines400].flatMap(x=>[{op:'toFixed',x,n:2},{op:'toFixed',x,n:2,mode:3},{op:'toFixed',x,n:0,mode:2}]),
  ...[[huge,'2e'+nines400],[huge,'-3e'+nines400],['-1.5e'+nines400,'2.5e'+nines400],[tiny,'3e-'+nines400],[tiny,'-1e-'+nines400]]
    .flatMap(([x,y])=>[{op:'plus',x,y},{op:'minus',x,y}]),
  {op:'times',x:huge,y:tiny,dp:2},{op:'sqrt',x:'4e1000000000000000000000'},{op:'sqrt',x:'4e-1000000000000000000000'},
])
// JSON null selects the default for dp, rm and repeat, as the adapter's ?? does.
add('null-options','JSON null decimal places and rounding mode select the defaults 20 and 1',[
  {op:'div',x:'1',y:'3',dp:null},{op:'div',x:'2',y:'3',rm:null},{op:'div',x:'2',y:'3',dp:null,rm:null},{op:'div',x:'1',y:'0',dp:null},
  {op:'div',x:'0',y:'3',dp:null},{op:'div',x:'1',y:'',dp:null},{op:'sqrt',x:'2',dp:null},{op:'sqrt',x:'2',rm:null},
  {op:'pow',x:'3',n:-3,dp:null,rm:null},{op:'round',x:'2.5',rm:null},{op:'toFixed',x:'2.345',n:2,rm:null},{op:'toPrecision',x:'2.345',n:3,dp:null,rm:null},
])
corpus.push({id:'null-repeat',note:'repeat: null runs the cases once',input:{cases:[{op:'plus',x:'1',y:'2'},{op:'sqrt',x:'2',dp:10}],repeat:null}})
// NUMERIC is /^-?(\d+(\.\d*)?|\.\d+)(e[+-]?\d+)?$/i without the u flag: \d is ASCII, $ is the end of
// input, nothing is trimmed, and only e and E fold.
add('grammar','ASCII digits only, no trimming, no line-terminator $, no other numeric syntax',
  ['1\n','\n1','1\r','\r\n1','\t1','1\t','\u000b1','\u000c1',' 1','1 ','﻿1',' 1','1 ','　1','᠎1',
    '١٢٣','１','1０','०','1e١','1e1.5','0x10','0b1','0o7','1_000','1,5','1e','1e+','1e-','e1','.e1','-',
    '-.','.','0.','-0.','-.5e-3','+.5','+1','1..2','1e1e1','1e++1','--1','-+1','1-','00','-00.000e+99','1E5','1K',
    'ı','Infinity','-Infinity','NaN','1n','1e5\n','1.5E-7']
    .flatMap(x=>[{op:'toString',x},{op:'plus',x:'1',y:x}]))
// The order of upstream's checks: operand parse, then decimal places, then division by zero, then the
// rounding mode, which round() validates only when it rounds.
add('error-order','Which error wins when several apply',[
  {op:'div',x:'',y:'0',dp:-1,rm:9},{op:'div',x:'1',y:'',dp:-1,rm:9},{op:'div',x:'1',y:'0',dp:-1,rm:9},{op:'div',x:'0',y:'0',rm:9},
  {op:'div',x:'0',y:'3',rm:9},{op:'div',x:'1',y:'4',rm:9},{op:'div',x:'1',y:'3',rm:9},{op:'div',x:'1',y:'3',dp:0.5},{op:'div',x:'6',y:'3',dp:2**32},
  {op:'pow',x:'',n:0.5},{op:'pow',x:'2',n:0.5,dp:-1},{op:'pow',x:'2',n:-1,dp:-1,rm:9},{op:'pow',x:'0',n:-1,rm:9},{op:'pow',x:'0',n:-1,dp:-1},
  {op:'pow',x:'2',n:-2,rm:9},{op:'pow',x:'3',n:-1,rm:9},{op:'pow',x:'2',n:3,dp:-1,rm:9},{op:'pow',x:'2'},
  {op:'sqrt',x:'-1',dp:-5,rm:9},{op:'sqrt',x:'0',dp:-5,rm:9},{op:'sqrt',x:'-0',dp:0.5},{op:'sqrt',x:'',dp:-5},
  {op:'round',x:'1.5',n:0.5,mode:9},{op:'round',x:'1.5',n:1,mode:9},{op:'round',x:'1.5',mode:9},{op:'round',x:'1.5',rm:9},{op:'round',x:'1.5',rm:9,mode:1},
  {op:'toFixed',x:'1.5',mode:9},{op:'toFixed',x:'1.5',n:-1,mode:9},{op:'toFixed',x:'1.5',n:0,mode:9},{op:'toFixed',x:'1.5',rm:9},{op:'toFixed',x:'1.5',n:0,rm:9},
  {op:'toExponential',x:'1.5',mode:9},{op:'toExponential',x:'1.5',n:0,mode:9},{op:'toPrecision',x:'1.5',mode:9},{op:'toPrecision',x:'1.5',n:0,mode:9},
  {op:'toPrecision',x:'1.5',n:1,mode:9},{op:'toString',x:'1.5',dp:-1,rm:9},{op:'plus',x:'1',y:'2',dp:-1,rm:9},{op:'cmp',x:'1',y:'',rm:9},
  {op:'times',x:'x',y:''},{op:'nope',x:'1'},{op:'nope',x:''},
])
// Options past int32 and past 2^53 are integers that fail big.js's ~~ and range checks; ports must
// not wrap or saturate them into range.
add('wide-options','Decimal places, precisions, exponents and rounding modes past int32 and 2^53',[
  ...[2**31,2**31+1,2**32,2**32+1,2**32+2,2**53,2**53+2,1e21,-(2**31),-(2**32)-1].flatMap(n=>[
    {op:'round',x:'1.25',n},{op:'toFixed',x:'1.25',n},{op:'toExponential',x:'1.25',n},{op:'toPrecision',x:'1.25',n},{op:'pow',x:'2',n},
    {op:'div',x:'1',y:'3',dp:n},{op:'sqrt',x:'2',dp:n},{op:'div',x:'1',y:'3',rm:n},{op:'round',x:'1.25',n:1,mode:n},{op:'round',x:'1.25',n:1,mode:n+1}]),
  {op:'pow',x:'1',n:1000000},{op:'pow',x:'-1',n:-1000000},{op:'pow',x:'1',n:-1000000},{op:'pow',x:'1',n:1000001},{op:'round',x:'1.25',n:-1000000},{op:'round',x:'1.25',n:1000000},
  {op:'toFixed',x:'1.25',n:1000001},{op:'toPrecision',x:'1.25',n:1000001},{op:'div',x:'1',y:'4',dp:1000000},{op:'div',x:'1',y:'4',dp:1000001},
])
// JSON numbers go through String(); big.js maps -0 to '-0', and zero signs are never printed.
add('json-numbers','Number operands at the boundary',
  [1e21,1e-7,1e-6,5e-324,0.1+0.2,123456789012345680000,2**53+2,1.7976931348623157e308,-1e-7,1e300,4.35e-321,-0.5,123.456,2**31,-(2**32)]
    .flatMap(x=>['toString','plus','div','sqrt','cmp'].map(op=>({op,x,y:x,dp:30}))))
// Wide operands: linear passes over 20,000 digits and quadratic ones over a few thousand, against
// quadratic string building and deep recursion in the ports.
{
  const big = decimal(20000,7), big2 = decimal(20000,99)
  add('wide-operands','20,000-digit coefficients and thousand-digit products, quotients and roots',[
    {op:'plus',x:big,y:big2},{op:'minus',x:big,y:'-'+big2+'e-10000'},{op:'minus',x:big,y:big},{op:'cmp',x:big,y:big2},{op:'cmp',x:big,y:big+'1'},
    {op:'toFixed',x:big+'e-12000',n:6000,mode:3},{op:'round',x:big+'e-12000',n:10,mode:2},{op:'toString',x:'0.'+'0'.repeat(20000)+'1'},
    {op:'toString',x:big+'e-1'},{op:'toFixed',x:'1e-15000',n:15001},{op:'toExponential',x:big,n:19990,mode:1},{op:'toPrecision',x:big,n:19999,mode:0},
    {op:'times',x:decimal(2000,3),y:decimal(2000,5)},{op:'div',x:decimal(4000,11),y:decimal(300,3),dp:3000},{op:'div',x:'1',y:decimal(1500,17),dp:2000,rm:2},
    {op:'sqrt',x:decimal(600,19),dp:500},{op:'sqrt',x:decimal(41,23)+'e-1000',dp:1100,rm:3},{op:'pow',x:'1.0000001',n:2000,dp:20},{op:'pow',x:'1.0000001',n:-300,dp:500},
  ])
}
const fifty=decimal(50,37), other=decimal(50,81)
add('bench-mixed','10,000 mixed operations on 50-digit coefficients',
  ['plus','minus','times','div','cmp','toString','round','toFixed','toExponential','toPrecision'].map(op=>({op,x:fifty,y:other,n:9})),1000)
writeFileSync(new URL('corpus.json',import.meta.url),JSON.stringify({schema:1,library:'big-js',cases:corpus})+'\n')
