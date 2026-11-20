/* BLAKE2b uses two 32-bit limbs per word. BLAKE2s uses just the low limb. */
type word = {lo: int, hi: int}
let ivLow = [-205731576, -2067093701, -23791573, 1595750129, -1377402159, 725511199, -79577749, 327033209]
let ivHigh = [1779033703, -1150833019, 1013904242, -1521486534, 1359893119, -1694144372, 528734635, 1541459225]
let sigma = [
  [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
  [14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3],
  [11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4],
  [7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8],
  [9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13],
  [2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9],
  [12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11],
  [13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10],
  [6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5],
  [10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0]
]

let xorBits = Int.bitwiseXor
let orBits = Int.bitwiseOr
let shl = Int.shiftLeft
let shr = Int.shiftRightUnsigned
let at = Array.getUnsafe

let add = (a, b) => {
  let lo = a.lo + b.lo
  let carry = xorBits(lo, -2147483648) < xorBits(a.lo, -2147483648) ? 1 : 0
  {lo, hi: a.hi + b.hi + carry}
}

let xorWord = (a, b) => {lo: xorBits(a.lo, b.lo), hi: xorBits(a.hi, b.hi)}
let combine = (lo, hi, n) => orBits(shr(lo, n), shl(hi, 32 - n))
let rotate = (w, n, wide) =>
  if !wide { {lo: combine(w.lo, w.lo, n), hi: 0} }
  else if n == 32 { {lo: w.hi, hi: w.lo} }
  else {
    let lo = n > 32 ? w.hi : w.lo
    let hi = n > 32 ? w.lo : w.hi
    let bits = Int.bitwiseAnd(n, 31)
    {lo: combine(lo, hi, bits), hi: combine(hi, lo, bits)}
  }
let initial = (wide, i) => wide ? {lo: at(ivLow, i), hi: at(ivHigh, i)} : {lo: at(ivHigh, i), hi: 0}
let load32 = (bytes, pos) =>
  orBits(orBits(at(bytes,pos), shl(at(bytes,pos+1),8)), orBits(shl(at(bytes,pos+2),16), shl(at(bytes,pos+3),24)))
let load = (wide, bytes, i) => wide
  ? {lo: load32(bytes,8*i), hi: load32(bytes,8*i+4)}
  : {lo: load32(bytes,4*i), hi: 0}

let mix = (wide, v, a, b, c, d, x, y) => {
  let (r1,r2,r3,r4) = wide ? (32,24,16,63) : (16,12,8,7)
  v[a] = add(add(at(v,a),at(v,b)),x)
  v[d] = rotate(xorWord(at(v,d),at(v,a)),r1,wide)
  v[c] = add(at(v,c),at(v,d))
  v[b] = rotate(xorWord(at(v,b),at(v,c)),r2,wide)
  v[a] = add(add(at(v,a),at(v,b)),y)
  v[d] = rotate(xorWord(at(v,d),at(v,a)),r3,wide)
  v[c] = add(at(v,c),at(v,d))
  v[b] = rotate(xorWord(at(v,b),at(v,c)),r4,wide)
}

let compress = (wide, h, block, count, last) => {
  let v = Array.fromInitializer(~length=16, i => i < 8 ? at(h,i) : initial(wide,i-8))
  let msg = Array.fromInitializer(~length=16, i => load(wide,block,i))
  v[12] = xorWord(at(v,12),{lo:count.lo,hi: wide ? count.hi : 0})
  if !wide { v[13] = xorWord(at(v,13),{lo:count.hi,hi:0}) }
  if last { v[14] = xorWord(at(v,14),{lo:-1,hi:-1}) }
  let rounds = wide ? 12 : 10
  for r in 0 to rounds-1 {
    let s = at(sigma,Int.mod(r,10))
    for i in 0 to 3 {
      mix(wide,v,i,i+4,i+8,i+12,at(msg,at(s,2*i)),at(msg,at(s,2*i+1)))
    }
    for i in 0 to 3 {
      mix(wide,v,i,4+Int.mod(i+1,4),8+Int.mod(i+2,4),12+Int.mod(i+3,4),
        at(msg,at(s,8+2*i)),at(msg,at(s,9+2*i)))
    }
  }
  for i in 0 to 7 { h[i] = xorWord(at(h,i),xorWord(at(v,i),at(v,i+8))) }
}

let digest = (wide, chunks, dkLen, key, salt, personalization) => {
  let stride = wide ? 8 : 4
  let blockLen = stride*16
  let h = Array.fromInitializer(~length=8, i => initial(wide,i))
  let param = orBits(0x01010000,orBits(shl(Array.length(key),8),dkLen))
  h[0] = xorWord(at(h,0),{lo:param,hi:0})
  for i in 0 to 1 {
    if Array.length(salt)>0 {h[i+4] = xorWord(at(h,i+4),load(wide,salt,i))}
    if Array.length(personalization)>0 {h[i+6] = xorWord(at(h,i+6),load(wide,personalization,i))}
  }
  let block = Array.make(~length=blockLen,0)
  for i in 0 to Array.length(key)-1 {block[i] = at(key,i)}
  let pos = ref(Array.length(key)>0 ? blockLen : 0)
  let count = ref({lo:pos.contents,hi:0})
  chunks->Array.forEach(chunk => {
    let offset = ref(0)
    while offset.contents < Array.length(chunk) {
      if pos.contents == blockLen {compress(wide,h,block,count.contents,false); pos := 0}
      let take = Math.Int.min(blockLen-pos.contents,Array.length(chunk)-offset.contents)
      for i in 0 to take-1 {block[pos.contents+i] = at(chunk,offset.contents+i)}
      pos := pos.contents+take
      offset := offset.contents+take
      count := add(count.contents,{lo:take,hi:0})
    }
  })
  for i in pos.contents to blockLen-1 {block[i]=0}
  compress(wide,h,block,count.contents,true)
  Array.fromInitializer(~length=dkLen,i=>{
    let w = at(h,i/stride)
    let limb = Int.mod(i,stride)>=4 ? w.hi : w.lo
    Int.bitwiseAnd(shr(limb,8*Int.mod(i,4)),255)
  })
}
