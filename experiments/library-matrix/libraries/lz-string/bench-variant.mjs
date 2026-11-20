// Rename letters bijectively. Every repetition retains the same dictionary matches, UTF-16
// length and literal widths. Leave JSON syntax, numeric fields and Unicode escapes intact.
const letters = Array.from('GHIJKLMNOPQRSTUVWXYZghijklmnopqrstvwxyz', c => c.charCodeAt(0))
const mutable = new Set(letters)

export function variant(input, k) {
  const available = letters.slice()
  const names = new Map()
  const data = input.data.map(code => {
    if (!mutable.has(code)) return code
    if (!names.has(code)) {
      const index = k % available.length
      k = Math.floor(k / available.length)
      names.set(code, available.splice(index, 1)[0])
    }
    return names.get(code)
  })
  return {...input, data}
}
