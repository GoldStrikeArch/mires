// The npm module selects its embedded WebAssembly arithmetic during import.
import Long from '../../node_modules/long/index.js'

function mixed(L, a, b, rounds, radix) {
  const multiplier = b.or(L.UONE)
  let checksum = 0
  for (let i = 0; i < rounds; i++) {
    a = a.multiply(multiplier).add(L.fromInt(i + 1)).rotl(11).xor(b)
    const divisor = L.fromInt(3 + (i & 255), a.unsigned)
    a = a.add(a.divide(divisor))
    if ((i & 7) === 0) {
      const text = a.toString(radix)
      checksum = (Math.imul(checksum, 33) + text.length + text.charCodeAt(0)) | 0
    }
  }
  return a.toString(radix) + ':' + checksum
}

export function evaluate(L, input) {
  return input.cases.map(c => {
    const a = L.fromBits(c.a[0], c.a[1], c.u)
    const b = L.fromBits(c.b[0], c.b[1], c.v)
    try {
      let value
      switch (c.op) {
        case 'fromString': value = L.fromString(c.s, c.u, c.n); break
        case 'fromBits': case 'toString': value = a; break
        case 'add': value = a.add(b); break
        case 'subtract': value = a.subtract(b); break
        case 'multiply': value = a.multiply(b); break
        case 'divide': value = a.divide(b); break
        case 'modulo': value = a.modulo(b); break
        case 'and': value = a.and(b); break
        case 'or': value = a.or(b); break
        case 'xor': value = a.xor(b); break
        case 'shiftLeft': value = a.shiftLeft(c.n); break
        case 'shiftRight': value = a.shiftRight(c.n); break
        case 'shiftRightUnsigned': value = a.shiftRightUnsigned(c.n); break
        case 'rotl': value = a.rotl(c.n); break
        case 'clz': return String(a.clz())
        case 'ctz': return String(a.ctz())
        case 'compare': return String(a.compare(b))
        case 'mix': return mixed(L, a, b, c.n, c.r)
        default: throw new Error('unknown Long operation')
      }
      return value.toString(c.r)
    } catch {
      return '!'
    }
  }).join('\n')
}

export const run = input => evaluate(Long, input)
