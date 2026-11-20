// Synchronous transport between the matrix harness and the Elm worker.
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
  if ('error' in response) throw new TypeError(response.error)
  const result = response.ok
  if (result && result.vertices) {
    // Elm ports transport JSON values. Each null encodes an absent coordinate.
    const vertices = result.vertices
    for (let i = 0; i < vertices.length; i++) {
      if (vertices[i] === null) vertices[i] = undefined
    }
  }
  return result
}
