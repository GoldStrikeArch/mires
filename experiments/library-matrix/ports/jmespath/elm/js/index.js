// JS host of the Elm port: host I/O and the two JS operations Elm lacks, no jmespath logic.
//
// The input document goes to the program as postfix instructions (a primitive, or
// { array: n, token } / { object: keys, token } to build from the values before it), written with an
// explicit stack, so no nesting depth recurses here. Each array and object carries itself as its
// identity token. The result comes back the same way ({ undefined: true } for undefined) and is
// rebuilt with a stack.
//
// While evaluating, the program may ask for `a.localeCompare(b)` (max and min of strings) or
// whether a token is `===` to any of some others (contains); the host answers through the
// `collated` and `identical` ports. The builder's packaging makes port delivery synchronous
// (languages/elm/build.mjs), so each answer's next request lands in `reply` before send returns.
//
// Module state, as in the other Elm hosts: `worker` is the one Platform.worker instance, whose
// model is idle between calls, and `reply` is the slot its synchronous answer lands in.
import Elm from 'elm-program'

let worker
let reply

export function run(input) {
  if (!worker) {
    worker = Elm.Main.init()
    worker.ports.response.subscribe((value) => { reply = value })
  }
  reply = undefined
  worker.ports.request.send({ data: toOps(input.data), queries: input.queries })
  while (reply && (reply.collate || reply.identical)) {
    const { collate, identical } = reply
    reply = undefined
    if (collate) worker.ports.collated.send(Math.sign(collate[0].localeCompare(collate[1])))
    else worker.ports.identical.send(identical.slice(1).includes(identical[0]))
  }
  if (!reply || 'error' in reply) throw new Error(reply?.error ?? 'missing Elm response')
  return fromOps(reply.value)
}

function toOps(value) {
  const ops = []
  const work = [{ value }]
  while (work.length) {
    const { value: v, keys } = work.pop()
    if (keys) ops.push(Array.isArray(v) ? { array: v.length, token: v } : { object: keys, token: v })
    else if (Array.isArray(v)) {
      work.push({ value: v, keys: true })
      for (let i = v.length - 1; i >= 0; i--) work.push({ value: v[i] })
    } else if (v !== null && typeof v === 'object') {
      const own = Object.keys(v)
      work.push({ value: v, keys: own })
      for (let i = own.length - 1; i >= 0; i--) work.push({ value: v[own[i]] })
    } else ops.push(v)
  }
  return ops
}

function fromOps(ops) {
  const stack = []
  for (const op of ops) {
    if (op === null || typeof op !== 'object') stack.push(op)
    else if ('undefined' in op) stack.push(undefined)
    else if ('array' in op) stack.push(stack.splice(stack.length - op.array, op.array))
    else {
      const values = stack.splice(stack.length - op.object.length, op.object.length)
      stack.push(Object.fromEntries(op.object.map((key, i) => [key, values[i]])))
    }
  }
  return stack[0]
}
