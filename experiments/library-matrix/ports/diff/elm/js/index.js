import Elm from 'elm-program'
let app
let reply

// JSON ports cannot carry undefined. The patch encoder marks its optional fields.
function restore(value) {
  if (value === null || typeof value !== 'object') return value
  if (value.$diffUndefined === true) return undefined
  for (const key of Object.keys(value)) value[key] = restore(value[key])
  return value
}

export function run(input) {
  if (!app) {
    app = Elm.Main.init()
    app.ports.response.subscribe(value => { reply = value })
  }
  reply = undefined
  app.ports.request.send(input)
  if (!reply || reply.error) throw new TypeError(reply?.error || 'missing diff response')
  return restore(reply.ok)
}
