open Bindings

let getContrastPreference = (): any => {
  if asBool(get(matchMedia("(prefers-contrast: no-preference)"), "matches")) {
    identity(0)
  } else if (
    asBool(get(matchMedia("(prefers-contrast: high)"), "matches")) ||
      asBool(get(matchMedia("(prefers-contrast: more)"), "matches"))
  ) {
    identity(1)
  } else if (
    asBool(get(matchMedia("(prefers-contrast: low)"), "matches")) ||
      asBool(get(matchMedia("(prefers-contrast: less)"), "matches"))
  ) {
    identity(-1)
  } else if asBool(get(matchMedia("(prefers-contrast: forced)"), "matches")) {
    identity(10)
  } else {
    jsUndefined
  }
}
