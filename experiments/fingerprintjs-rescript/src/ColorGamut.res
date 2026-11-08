open Bindings

let getColorGamut = () => {
  let rec loop = i => {
    let gamuts = ["rec2020", "p3", "srgb"]
    if i >= arrayLength(gamuts) {
      jsUndefined
    } else {
      let gamut = getInt(gamuts, i)
      if asBool(get(matchMedia("(color-gamut: " ++ gamut ++ ")"), "matches")) {
        identity(gamut)
      } else {
        loop(i + 1)
      }
    }
  }
  loop(0)
}
