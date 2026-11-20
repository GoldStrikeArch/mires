export function variant(input, k) {
  const records = input.data.items
  const nonce = k.toString(36).padStart(11, '0')
  const items = records.map((_, i) => {
    const row = records[(i + k) % records.length]
    return { ...row, name: row.name + nonce, detail: row.detail + nonce }
  })
  return { data: { items }, queries: input.queries }
}
