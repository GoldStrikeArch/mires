// The parts of noble's utils.ts the selected hashes use, and the byte-order reads and writes of
// its DataView and Uint32Array views.

// noble's anumber: `n` must be a safe integer >= 0. Values above the int range saturate.
let anumber = (n: float, title) =>
  if n >= 0. && n <= 9007199254740991. && n == Math.floor(n) {
    n > 2147483647. ? 2147483647 : Float.toInt(n)
  } else {
    JsError.RangeError.throwWithMessage(`"${title}" expected integer >= 0, got ${Float.toString(n)}`)
  }

// noble's rotr for 32-bit words.
let rotr = (word, shift) => word << 32 - shift ||| word >>> shift

// DataView.getUint32(offset, false).
let getUint32BE = (data, offset) =>
  data->Array.getUnsafe(offset) << 24 |||
  data->Array.getUnsafe(offset + 1) << 16 |||
  data->Array.getUnsafe(offset + 2) << 8 |||
  data->Array.getUnsafe(offset + 3)

// A little-endian 32-bit word, as noble's Uint32Array views read on little-endian hosts.
let getUint32LE = (data, offset) =>
  data->Array.getUnsafe(offset) |||
  data->Array.getUnsafe(offset + 1) << 8 |||
  data->Array.getUnsafe(offset + 2) << 16 |||
  data->Array.getUnsafe(offset + 3) << 24

// DataView.setUint32(offset, value, false).
let setUint32BE = (data, offset, value) =>
  for i in 0 to 3 {
    data->Array.setUnsafe(offset + i, value >>> 24 - 8 * i &&& 255)
  }

// The first `count` bytes of the little-endian encoding of `words`.
let wordsToBytesLE = (words, count) =>
  Array.fromInitializer(~length=count, i => words->Array.getUnsafe(i / 4) >>> 8 * mod(i, 4) &&& 255)

// noble's bytesToHex.
let bytesToHex = bytes => {
  let digits = "0123456789abcdef"
  bytes
  ->Array.map(b => digits->String.charAt(b >>> 4) ++ digits->String.charAt(b &&& 15))
  ->Array.join("")
}
