open Bindings
open Browser
open Async

let getIndexedDB = () =>
  if isTrident() || isEdgeHTML() {
    jsUndefined
  } else {
    let outcome = runAction(() => isTruthy(get(window(), "indexedDB")))
    if asBool(getInt(outcome, 0)) {
      getInt(outcome, 1)
    } else {
      identity(true)
    }
  }
