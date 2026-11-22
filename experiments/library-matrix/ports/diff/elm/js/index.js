// JS host of the Elm port: the only Elm/JS boundary is the `request` / `response` port pair of
// src/Main.elm. The request also carries JavaScript's `undefined`, which Elm cannot construct,
// so optional patch fields come back as exact `undefined` without a sentinel or post-pass walk.
import Elm from 'elm-program'

let app
let reply

export function run(input) {
  if (!app) {
    app = Elm.Main.init()
    app.ports.response.subscribe(value => { reply = value })
  }
  reply = undefined
  app.ports.request.send({ input, undefined })
  if (!reply || reply.error) throw new TypeError(reply?.error || 'missing diff response')
  return reply.ok
}
