// noble's hmac.ts: _HMAC over the hash that `create` makes.
let make = (create: unit => Hash.t, key): Hash.t => {
  let iHash = create()
  let blockLen = iHash.blockLen
  let pad = Array.make(~length=blockLen, 0)
  // A key longer than a block is replaced by its digest.
  let key = if Array.length(key) > blockLen {
    let long = create()
    long.update(key)
    long.digest()
  } else {
    key
  }
  key->Array.forEachWithIndex((b, i) => pad->Array.setUnsafe(i, b))
  let xorPad = mask => pad->Array.forEachWithIndex((b, i) => pad->Array.setUnsafe(i, b ^^^ mask))
  xorPad(0x36)
  iHash.update(pad)
  let oHash = create()
  // Undo the inner XOR and apply the outer one.
  xorPad(0x36 ^^^ 0x5c)
  oHash.update(pad)
  {
    blockLen,
    outputLen: iHash.outputLen,
    update: iHash.update,
    digest: () => {
      oHash.update(iHash.digest())
      oHash.digest()
    },
  }
}
