open Bindings

let isTransparencyReduced = (): any =>
  if asBool(get(matchMedia("(prefers-reduced-transparency: reduce)"), "matches")) {
    identity(true)
  } else if asBool(get(matchMedia("(prefers-reduced-transparency: no-preference)"), "matches")) {
    identity(false)
  } else {
    jsUndefined
  }
