open Bindings
open Data
open Browser

let getUnstableHardwareConcurrency = () =>
  replaceNaN(toInt(get(navigator(), "hardwareConcurrency")), jsUndefined)

let getHardwareConcurrency = () => {
  let value = getUnstableHardwareConcurrency()
  if !isUndefined(value) && isGecko() && isGecko143OrNewer() {
    if asInt(value) >= 8 {
      8
    } else {
      4
    }
  } else {
    value
  }
}
