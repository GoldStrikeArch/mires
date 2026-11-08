open Bindings

let isHDR = (): any =>
  if asBool(get(matchMedia("(dynamic-range: high)"), "matches")) {
    identity(true)
  } else if asBool(get(matchMedia("(dynamic-range: standard)"), "matches")) {
    identity(false)
  } else {
    jsUndefined
  }
