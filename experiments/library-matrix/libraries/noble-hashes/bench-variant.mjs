// Change the repeated message without changing its byte count or chunk boundaries.
export function variant(input, k) {
  const chunks = input.chunks.map(block => block.slice())
  for (let i = 0; i < 4; i++) chunks[0][i] ^= (k >>> (8 * i)) & 255
  return { ...input, chunks }
}
