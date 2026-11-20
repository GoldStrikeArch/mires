import Elm from 'elm-program'

let app
let reply

export function run(input) {
  if (!app) {
    app = Elm.Main.init()
    app.ports.response.subscribe(value => { reply = value })
  }
  reply = undefined
  app.ports.request.send(input)
  if (!reply || reply.error) throw new TypeError(reply?.error || 'missing codec response')
  return reply.ok
}
