import { writeFileSync } from 'node:fs'

const algorithms = ['sha256','sha512','sha3_256','keccak_256','blake2b','blake2s','hmac_sha256']
const cases = []
function add(id, input) { cases.push({ id, input }) }
function bytes(n, seed = 0) { return Array.from({ length: n }, (_, i) => (seed + i * 73 + (i >>> 3)) & 255) }
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
for (const algorithm of ['sha256','sha512','sha3_256']) {
  add('bench-'+algorithm,{algorithm,chunks:[bytes(4096,37)],repeat:256})
}
writeFileSync(new URL('corpus.json',import.meta.url), JSON.stringify({schema:1,library:'noble-hashes',cases})+'\n')
