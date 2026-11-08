open Bindings

let getMonochromeDepth = () => {
  if !asBool(get(matchMedia("(min-monochrome: 0)"), "matches")) {
    jsUndefined
  } else {
    let rec loop = i =>
      if i > 100 {
        throwJs(newError("Too high value"))
      } else if asBool(get(matchMedia("(max-monochrome: " ++ Int.toString(i) ++ ")"), "matches")) {
        identity(i)
      } else {
        loop(i + 1)
      }
    loop(0)
  }
}
