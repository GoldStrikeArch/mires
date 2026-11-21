// JS host of the Elm driver: the only Elm/JS boundary is the `request` / `response` port pair of
// src/Main.elm, and it does no Explorer work of its own. The linker sends each [input, history]
// packet; the driver answers with the next component request or the final snapshot. The builder's
// packaging makes outgoing port delivery synchronous (see languages/elm/build.mjs), so `run`
// returns the response of the packet it sent.
//
// Module state, as in the other Elm hosts: `worker` is the one Platform.worker instance, whose
// model is `()`, and `reply` is the slot its synchronous answer lands in.
import Elm from 'elm-program'

let worker
let reply

export function run(packet) {
  if (!worker) {
    worker = Elm.Main.init()
    worker.ports.response.subscribe((value) => {
      reply = value
    })
  }
  reply = undefined
  worker.ports.request.send(packet)
  if (reply === undefined) throw new Error('the Elm port did not respond synchronously')
  return reply
}
