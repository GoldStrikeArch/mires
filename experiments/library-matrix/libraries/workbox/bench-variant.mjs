// A distinct benchmark input per k (k >= 1): every URL and route prefix under the bench/ root moves
// to a k-named root of fixed length, and every revision gains the same k token, so no URL, cache
// key or revision repeats across calls while the work per call stays the same.
export function variant(input, k) {
  const token = k.toString(36).padStart(11, '0')
  const root = 'b' + token + '/'
  const rename = (value, key) => {
    if (typeof value === 'string') {
      if (key === 'revision') return value + '.' + token
      return value.startsWith('bench/') ? root + value.slice('bench/'.length) : value
    }
    if (Array.isArray(value)) return value.map(item => rename(item))
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([name, item]) => [name, rename(item, name)]))
    return value
  }
  return rename(input)
}
