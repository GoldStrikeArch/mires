import Elm from 'elm-program'

let worker
let reply

export function run(input) {
  if (!worker) {
    worker = Elm.Main.init()
    worker.ports.response.subscribe(value => { reply = value })
  }
  reply = undefined
  worker.ports.request.send(input)
  // Elm uses the host's locale for JavaScript String.localeCompare.
  while (reply?.collate || reply?.identity) {
    const { collate, identity } = reply
    reply = undefined
    if (collate) worker.ports.comparison.send(Math.sign(collate[0].localeCompare(collate[1])))
    else worker.ports.identityResult.send(identity[0] === identity[1])
  }
  if (!reply || 'error' in reply) throw new Error(reply?.error ?? 'missing Elm response')
  return fromWire(reply.value)
}

function fromWire(value) {
  if (Array.isArray(value)) return value.map(fromWire)
  if (value && typeof value === 'object') {
    if (value.missing) return undefined
    return Object.fromEntries(value.entries.map(([k, v]) => [k, fromWire(v)]))
  }
  return value
}
