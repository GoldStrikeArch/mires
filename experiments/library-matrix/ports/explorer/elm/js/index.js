import Elm from 'elm-program'

let worker
let reply
export function run(packet) {
  if (!worker) {
    worker = Elm.Main.init()
    worker.ports.response.subscribe(value => { reply = value })
  }
  worker.ports.request.send(packet)
  return reply
}
