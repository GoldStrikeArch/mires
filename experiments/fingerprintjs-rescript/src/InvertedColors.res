open Bindings

let areColorsInverted = (): any =>
  if asBool(get(matchMedia("(inverted-colors: inverted)"), "matches")) {
    identity(true)
  } else if asBool(get(matchMedia("(inverted-colors: none)"), "matches")) {
    identity(false)
  } else {
    jsUndefined
  }
