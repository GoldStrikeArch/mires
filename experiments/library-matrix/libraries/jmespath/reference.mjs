import jmespath from '../../node_modules/jmespath/matrix-entry.mjs'

function category(e) {
  const msg = String(e.message)
  if (msg.startsWith('TypeError')) return 'invalid-type'
  if (msg.startsWith('ArgumentError')) return 'invalid-arity'
  if (msg.startsWith('Unknown function')) return 'unknown-function'
  if (e.name === 'RuntimeError') return 'invalid-value'
  return 'syntax'
}

export function run({ data, queries }) {
  return queries.map(({ op, expression }) => {
    try {
      return { value: op === 'compile' ? jmespath.compile(expression) : jmespath.search(data, expression) }
    } catch (e) {
      return { error: category(e) }
    }
  })
}
