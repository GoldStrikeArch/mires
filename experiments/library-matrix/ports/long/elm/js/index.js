// JS host of the Elm port: the only Elm/JS boundary is the `request` / `response` port pair of
// src/Main.elm. `elm-program` is the compiled Elm module, which the builder supplies. The
// builder's packaging makes outgoing port delivery synchronous (see languages/elm/build.mjs), so
// `run` returns the response of the request it sent.
import Elm from 'elm-program'

let app
let response

function instance() {
  if (!app) {
    app = Elm.Main.init()
    app.ports.response.subscribe((value) => {
      response = value
    })
  }
  return app
}

export function run(input) {
  response = undefined
  instance().ports.request.send(input)
  if (response === undefined) throw new Error('the Elm port did not respond synchronously')
  if ('error' in response) throw new Error(response.error)
  return response.ok
}
