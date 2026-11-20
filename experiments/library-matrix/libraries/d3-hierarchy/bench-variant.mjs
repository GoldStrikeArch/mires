export function variant(input, k) {
  const data = structuredClone(input.data)
  const pending = [data]
  // Low bits fit exactly in every fixture weight. Keep the remaining bits in
  // the node id so even the largest safe integer selects a distinct subtree.
  const high = Math.floor(k / 2 ** 32)
  const suffix = high ? '-' + high.toString(36) : ''
  const delta = ((k % 2 ** 32) + 1) / 2 ** 37
  while (pending.length) {
    const node = pending.pop()
    node.value += delta
    node.id += suffix
    if (node.children) pending.push(...node.children)
  }
  return {...input, data}
}
