// Keep each program's operation count and radix fixed while varying both operands.
export function variant(input, k) {
  // Multiplication by an odd constant permutes the low 32 nonce bits.
  // Keep the other 21 bits in the high words of the operands.
  const mix = Math.imul((k >>> 0) + 1, 0x9e3779b1)
  const high = Math.floor(k / 2 ** 32)
  return { cases: input.cases.map(c => ({
    ...c,
    a: [c.a[0] ^ mix, (c.a[1] + mix) ^ high],
    b: [c.b[0] ^ (mix >>> 3), c.b[1] ^ high],
  })) }
}
