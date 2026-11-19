// Bench variant for the pilot (registry `bench.variant`; tools/lib/protocol.mjs). The k-th variant
// (k >= 1) rotates the case's space-separated words by k positions and replaces the word at a
// k-chosen position with the nonce `v<k in base 36>`. Every prefix and every suffix of the text
// therefore changes with k, so no cache keyed on any unchanged part of the input (a prefix, a
// suffix, the text without its last word) can hit, while the work per call stays the same: the
// same words in another order, one of them replaced.
//
// Parity domain: replacing one space-separated word changes the token count by at most a few
// (the bench text has 782 tokens; its words carry 0 to 4 tokens each), so variants stay between
// 779 and 783 tokens, never a multiple of 16. Each variant's expected output comes from the
// reference on the same variant.
export function variant(input, index) {
  const words = input.text.split(' ')
  const rotation = index % words.length
  const rotated = words.slice(rotation).concat(words.slice(0, rotation))
  rotated[(index * 7919) % rotated.length] = `v${index.toString(36)}`
  return { text: rotated.join(' '), top: input.top }
}
