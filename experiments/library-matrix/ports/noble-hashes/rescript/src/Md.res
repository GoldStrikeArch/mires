// noble's _md.ts: the Merkle–Damgård base of SHA-256 and SHA-512, and its Chi and Maj.

// Bits of b where a is set, else of c.
let chi = (a, b, c) => a &&& b ^^^ ~~~a &&& c

// The majority of each bit of a, b and c.
let maj = (a, b, c) => a &&& b ^^^ a &&& c ^^^ b &&& c

// noble's HashMD. `process` compresses the block at an offset; `get` returns the state words,
// the high half before the low one for 64-bit words.
type t = {
  blockLen: int,
  outputLen: int,
  padOffset: int,
  buffer: array<int>,
  mutable pos: int,
  mutable length: float,
  process: (array<int>, int) => unit,
  get: unit => array<int>,
}

let make = (~blockLen, ~outputLen, ~padOffset, ~process, ~get) => {
  blockLen,
  outputLen,
  padOffset,
  buffer: Array.make(~length=blockLen, 0),
  pos: 0,
  length: 0.,
  process,
  get,
}

let update = (h, data) => {
  let len = Array.length(data)
  let pos = ref(0)
  while pos.contents < len {
    let take = Math.Int.min(h.blockLen - h.pos, len - pos.contents)
    if take == h.blockLen {
      // With nothing buffered, whole blocks are compressed straight from the input.
      while h.blockLen <= len - pos.contents {
        h.process(data, pos.contents)
        pos := pos.contents + h.blockLen
      }
    } else {
      for i in 0 to take - 1 {
        h.buffer->Array.setUnsafe(h.pos + i, data->Array.getUnsafe(pos.contents + i))
      }
      h.pos = h.pos + take
      pos := pos.contents + take
      if h.pos == h.blockLen {
        h.process(h.buffer, 0)
        h.pos = 0
      }
    }
  }
  h.length = h.length +. Int.toFloat(len)
}

let digest = h => {
  let {buffer, blockLen} = h
  buffer->Array.setUnsafe(h.pos, 0x80)
  let pos = h.pos + 1
  buffer->Array.fill(0, ~start=pos)
  // No room for the length field: pad this block and put the length in the next.
  if h.padOffset > blockLen - pos {
    h.process(buffer, 0)
    buffer->Array.fill(0)
  }
  // noble's setU64FromNum: only the low 64 bits of the length field; the rest stays zero.
  let bits = h.length *. 8.
  Utils.setUint32BE(buffer, blockLen - 8, U64.fromNumH(bits))
  Utils.setUint32BE(buffer, blockLen - 4, U64.fromNumL(bits))
  h.process(buffer, 0)
  let state = h.get()
  let out = Array.make(~length=h.outputLen, 0)
  for i in 0 to h.outputLen / 4 - 1 {
    Utils.setUint32BE(out, 4 * i, state->Array.getUnsafe(i))
  }
  out
}

let toHash = (h): Hash.t => {
  blockLen: h.blockLen,
  outputLen: h.outputLen,
  update: data => update(h, data),
  digest: () => digest(h),
}
