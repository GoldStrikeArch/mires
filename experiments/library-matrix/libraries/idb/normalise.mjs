// Firefox gives a module imported from a WebDriver script the records IndexedDB reads (and
// structuredClone results) as objects of another realm, which the json form's prototype check
// rejects. Copy plain records into this realm; values, key order, undefined and -0 are kept.
// Applied to the reference and the port alike (registry `normalisation`).
export function apply(value) {
  if (Array.isArray(value)) return Array.from(value, apply)
  if (value === null || typeof value !== 'object') return value
  const proto = Object.getPrototypeOf(value)
  if (proto !== null && (Object.getPrototypeOf(proto) !== null || proto.constructor?.name !== 'Object')) return value
  return Object.fromEntries(Object.keys(value).map((key) => [key, apply(value[key])]))
}
