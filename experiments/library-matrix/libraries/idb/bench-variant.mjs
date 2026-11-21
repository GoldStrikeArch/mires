// The bench script's keys, groups and values are strings such as k07, g3 and v12. Each call renames
// every one of them with the same suffix, which keeps their order and so the work per call.
export function variant(input, k) {
  const rename = (value) => {
    if (typeof value === 'string') return /^[kgv]\d+$/.test(value) ? value + '.' + k : value
    if (Array.isArray(value)) return value.map(rename)
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, rename(item)]))
    return value
  }
  return rename(input)
}
