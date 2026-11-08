open Bindings

let areColorsForced = (): any =>
  if asBool(get(matchMedia("(forced-colors: active)"), "matches")) {
    identity(true)
  } else if asBool(get(matchMedia("(forced-colors: none)"), "matches")) {
    identity(false)
  } else {
    jsUndefined
  }
