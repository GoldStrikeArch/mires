open Bindings
open Data
open Browser

let getUnstableScreenResolution = () => {
  let s = screen()
  let parseDimension = value => replaceNaN(toInt(value), jsNull)
  let dimensions = [parseDimension(get(s, "width")), parseDimension(get(s, "height"))]
  arraySortReverse(arraySort(dimensions))
}

let getScreenResolution = () =>
  if isWebKit() && isWebKit616OrNewer() && isSafariWebKit() {
    jsUndefined
  } else {
    identity(getUnstableScreenResolution())
  }
