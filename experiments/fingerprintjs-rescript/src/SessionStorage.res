open Bindings
open Async

let getSessionStorage = () => {
  let outcome = runAction(() => isTruthy(get(window(), "sessionStorage")))
  if asBool(getInt(outcome, 0)) {
    getInt(outcome, 1)
  } else {
    true
  }
}
