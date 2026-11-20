import { writeFileSync } from 'node:fs'

const algorithms = ['sha256','sha512','sha3_256','keccak_256','blake2b','blake2s','hmac_sha256']
const cases = []
function add(id, input) { cases.push({ id, input }) }
function bytes(n, seed = 0) { return Array.from({ length: n }, (_, i) => (seed + i * 73 + (i >>> 3)) & 255) }
// Decimal digits only: a large message whose JSON stays about two characters per byte.
function digits(n, seed = 0) { return Array.from({ length: n }, (_, i) => (seed + i * 7 + (i >>> 9)) % 10) }
for (const algorithm of algorithms) {
  const key = algorithm === 'hmac_sha256' ? { key: bytes(129) } : {}
  for (const n of [0,1,3,55,56,63,64,65,111,112,127,128,129,135,136,137,255,1024]) {
    const data = n === 3 ? [97,98,99] : bytes(n)
    add(algorithm + '-' + n, { algorithm, bytes: data, ...key })
    add(algorithm + '-chunks-' + n, { algorithm, chunks: [[],data.slice(0,1),data.slice(1,63),[],data.slice(63)], ...key })
  }
}
for (const [algorithm,max] of [['blake2b',64],['blake2s',32]]) {
  for (const n of [1,max-1,max]) {
    const options = { key: bytes(n), salt: bytes(max/4,3), personalization: bytes(max/4,4), dkLen:n }
    add(algorithm + '-keyed-' + n, {algorithm,bytes:bytes(255), ...options})
    add(algorithm + '-keyed-empty-' + n, {algorithm,chunks:[[],[]], ...options})
  }
  for (const [name, field, value] of [['short','dkLen',0],['long','dkLen',max+1],['fraction','dkLen',1.5],['empty-key','key',[]],['long-key','key',bytes(max+1)],['salt','salt',[]],['personal','personalization',bytes(max/4+1)]]) {
    add(algorithm + '-invalid-' + name, {algorithm,bytes:[],[field]:value})
  }
}
add('hmac-rfc4231-1',{algorithm:'hmac_sha256',key:Array(20).fill(11),bytes:Array.from('Hi There',c=>c.charCodeAt(0))})
add('hmac-empty-key',{algorithm:'hmac_sha256',key:[],chunks:[]})
add('hmac-missing-key',{algorithm:'hmac_sha256',bytes:[]})
add('unknown-algorithm',{algorithm:'unknown',bytes:[1]})

// Added by the 2026-11-20 review (review-20261120, item R-noble).

// Names npm does not export, or not under that spelling: all throw.
for (const [name, algorithm] of [['upper','SHA256'],['trailing-space','sha256 '],['leading-space',' sha256'],['sha384','sha384'],['sha224','sha224'],['hmac','hmac'],['blake2','blake2'],['to-string','toString'],['proto','__proto__'],['constructor','constructor'],['has-own','hasOwnProperty'],['empty','']]) {
  add('unknown-algorithm-' + name, {algorithm,bytes:[1]})
}
// npm ignores dkLen, key, salt and personalization on SHA-2 and SHA-3, even values BLAKE2 rejects.
const ignored = [['dklen',{dkLen:5}],['dklen-zero',{dkLen:0}],['dklen-fraction',{dkLen:1.5}],['dklen-negative-zero',{dkLen:-0}],['dklen-past-int32',{dkLen:2**32+1}],['dklen-past-2-53',{dkLen:2**53+2}],['empty-key',{key:[]}],['key',{key:[1,2]}],['salt',{salt:[1]}],['personalization',{personalization:[1,2,3]}]]
for (const algorithm of ['sha256','sha512','sha3_256','keccak_256']) {
  for (const [name, options] of ignored) add(algorithm + '-ignored-' + name, {algorithm,bytes:[1,2,3],...options})
  add(algorithm + '-ignored-chunks', {algorithm,chunks:[[1],[2,3]],dkLen:1000,salt:bytes(3)})
}
// HMAC ignores dkLen, salt and personalization; keys around the 64-byte block and the 32-byte digest.
for (const [name, options] of [['dklen',{dkLen:3}],['dklen-fraction',{dkLen:1.5}],['salt',{salt:[1,2]}],['personalization',{personalization:[]}]]) {
  add('hmac-ignored-' + name, {algorithm:'hmac_sha256',key:[5],bytes:[1],...options})
}
for (const n of [1,31,32,33,63,64,65,128]) add('hmac-key-' + n, {algorithm:'hmac_sha256',key:bytes(n,n),chunks:[bytes(70),bytes(3)]})
add('hmac-missing-key-chunks', {algorithm:'hmac_sha256',chunks:[[1]]})
// BLAKE2 digest lengths past int32 and past 2^53, negative zero and fractions; error order with
// several invalid options; keyed messages of one and two blocks; salt or personalization alone.
for (const [algorithm,max] of [['blake2b',64],['blake2s',32]]) {
  for (const [name, dkLen] of [['2',2],['3',3],['5',5],['7',7],['9',9],['negative-zero',-0],['negative',-1],['half',0.5],['max-half',max+0.5],['2-31',2**31],['2-32',2**32],['2-32-plus-1',2**32+1],['2-32-plus-max',2**32+max],['minus-2-32-plus-1',1-2**32],['2-53',2**53],['2-53-plus-2',2**53+2],['1e21',1e21]]) {
    add(algorithm + '-dklen-' + name, {algorithm,bytes:[9,9],dkLen})
  }
  add(algorithm + '-invalid-dklen-and-key', {algorithm,bytes:[],dkLen:0,key:[]})
  add(algorithm + '-invalid-dklen-and-salt', {algorithm,bytes:[],dkLen:max+1,salt:[]})
  add(algorithm + '-invalid-key-and-personal', {algorithm,bytes:[],key:bytes(max+1),personalization:[]})
  for (const n of [max, 2*max, 2*max+1]) {
    add(algorithm + '-keyed-' + n + '-bytes', {algorithm,bytes:bytes(n,5),key:bytes(max,6)})
    add(algorithm + '-keyed-chunks-' + n, {algorithm,chunks:[bytes(n-1,5),bytes(1,7)],key:bytes(3),dkLen:max-1})
  }
  add(algorithm + '-salt-only', {algorithm,bytes:bytes(max),salt:bytes(max/4,8)})
  add(algorithm + '-personal-only', {algorithm,chunks:[bytes(max),[]],personalization:bytes(max/4,9)})
  add(algorithm + '-empty-chunks-repeat', {algorithm,chunks:[[]],repeat:4})
}
// One-shot bytes win over chunks, and repeat applies to chunks only.
add('sha256-bytes-and-chunks', {algorithm:'sha256',bytes:[1,2],chunks:[[3]],repeat:5})
add('sha512-bytes-repeat', {algorithm:'sha512',bytes:bytes(200),repeat:3})
add('sha256-no-chunks', {algorithm:'sha256',chunks:[]})
// Large inputs: a 2 MiB message in one update, 2 MiB messages streamed in 4,099-byte updates,
// 200,000 updates of 0 to 3 bytes on one state, and a chunks array 200,000 entries wide.
add('large-bytes-2mib-blake2b', {algorithm:'blake2b',bytes:digits(2*1024*1024+3,1)})
for (const algorithm of algorithms) {
  const key = algorithm === 'hmac_sha256' ? { key: bytes(20,2) } : {}
  add('large-stream-2mib-' + algorithm, {algorithm,chunks:[bytes(4099,37)],repeat:512,...key})
  add('stream-200k-updates-' + algorithm, {algorithm,chunks:[[],[7],[8,9],[10,11,12]],repeat:50000,...key})
}
add('wide-200k-chunks-hmac_sha256', {algorithm:'hmac_sha256',key:bytes(65,3),chunks:Array.from({length:200000},(_,i)=>digits(i%4,i))})

for (const algorithm of ['sha256','sha512','sha3_256']) {
  add('bench-'+algorithm,{algorithm,chunks:[bytes(4096,37)],repeat:256})
}
// JSON.stringify writes -0 as 0, so the negative-zero digest lengths above pass through a marker.
const text = JSON.stringify({schema:1,library:'noble-hashes',cases}, (_, value) => Object.is(value, -0) ? 'NEGATIVE_ZERO' : value)
writeFileSync(new URL('corpus.json',import.meta.url), text.replaceAll('"NEGATIVE_ZERO"', '-0')+'\n')
