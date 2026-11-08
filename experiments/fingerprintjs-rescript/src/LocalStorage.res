open Bindings
open Async

let getLocalStorage = () => {
  let outcome = runAction(() => isTruthy(get(window(), "localStorage")))
  if asBool(getInt(outcome, 0)) {
    getInt(outcome, 1)
  } else {
    true
  }
}
