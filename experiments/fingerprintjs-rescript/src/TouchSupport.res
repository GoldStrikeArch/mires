open Bindings
open Data
open Async

let getTouchSupport = () => {
  let n = navigator()
  let maxTouchPoints = if !isUndefined(get(n, "maxTouchPoints")) {
    identity(toInt(get(n, "maxTouchPoints")))
  } else if !isUndefined(get(n, "msMaxTouchPoints")) {
    get(n, "msMaxTouchPoints")
  } else {
    identity(0)
  }
  let outcome = runAction(() => {
    createEvent(document(), "TouchEvent")->ignore
    true
  })
  let touchEvent = if asBool(getInt(outcome, 0)) {
    getInt(outcome, 1)
  } else {
    false
  }
  let result = emptyObject()
  set(result, "maxTouchPoints", maxTouchPoints)
  set(result, "touchEvent", touchEvent)
  set(result, "touchStart", hasKey(window(), "ontouchstart"))
  result
}
