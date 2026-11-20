// A search can return an object from the WebDriver input realm verbatim.
// Copy plain data objects into this realm before the harness checks prototypes.
export function apply(value) {
  if (Array.isArray(value)) return Array.from(value, apply)
  if (value === null || typeof value !== 'object') return value
  const proto = Object.getPrototypeOf(value)
  if (proto !== null && (Object.getPrototypeOf(proto) !== null || proto.constructor?.name !== 'Object')) return value
  return Object.fromEntries(Object.keys(value).map(key => [key, apply(value[key])]))
}
