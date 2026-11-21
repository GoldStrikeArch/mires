import jmespath from '../../node_modules/jmespath/matrix-entry.mjs'

// Stack exhaustion is the engine's failure, not a JMESPath error: a RangeError ("Maximum call stack
// size exceeded") in V8 and JavaScriptCore, an InternalError ("too much recursion") in SpiderMonkey.
// dart2js recognises it by the same two tests.
function stackExhausted(e) {
  return (e instanceof RangeError && e.message.includes('call stack')) || (e?.name === 'InternalError' && e.message.includes('too much recursion'))
}

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
      // An exhausted stack propagates out of run, so the check fails instead of recording a category.
      if (stackExhausted(e)) throw e
      return { error: category(e) }
    }
  })
}
