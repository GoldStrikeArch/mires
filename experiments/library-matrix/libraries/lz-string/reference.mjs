import LZString from '../../node_modules/lz-string/matrix-entry.mjs'

const methods = {
  raw: ['compress', 'decompress'],
  base64: ['compressToBase64', 'decompressFromBase64'],
  utf16: ['compressToUTF16', 'decompressFromUTF16'],
  uri: ['compressToEncodedURIComponent', 'decompressFromEncodedURIComponent'],
  bytes: ['compressToUint8Array', 'decompressFromUint8Array'],
}

function fromUnits(units) {
  let value = ''
  for (let i = 0; i < units.length; i += 4096) value += String.fromCharCode(...units.slice(i, i + 4096))
  return value
}

export function units(value) {
  return Array.from({ length: value.length }, (_, i) => value.charCodeAt(i))
}

export function run({ op, format, data, repeat = 1 }) {
  const [encode, decode] = methods[format]
  if (op === 'decompress') {
    const input = data == null ? null : format === 'bytes' ? Uint8Array.from(data) : fromUnits(data)
    const result = LZString[decode](input)
    return result === null ? null : units(result)
  }
  const input = data == null ? null : fromUnits(data).repeat(repeat)
  const packed = LZString[encode](input)
  if (op === 'roundtrip') {
    const restored = LZString[decode](packed)
    if (restored !== input) throw new Error('roundtrip mismatch')
    return [packed.length, restored.length]
  }
  if (op !== 'compress') throw new TypeError('unknown operation')
  return format === 'bytes' ? Array.from(packed) : units(packed)
}
