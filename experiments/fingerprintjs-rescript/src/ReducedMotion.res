open Bindings

let isMotionReduced = (): any =>
  if asBool(get(matchMedia("(prefers-reduced-motion: reduce)"), "matches")) {
    identity(true)
  } else if asBool(get(matchMedia("(prefers-reduced-motion: no-preference)"), "matches")) {
    identity(false)
  } else {
    jsUndefined
  }
