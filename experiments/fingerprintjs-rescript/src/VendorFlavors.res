open Bindings

let getVendorFlavors = () => {
  let flavors = []
  let keys = [
    "chrome",
    "safari",
    "__crWeb",
    "__gCrWeb",
    "yandex",
    "__yb",
    "__ybro",
    "__firefox__",
    "__edgeTrackingPreventionStatistics",
    "webkit",
    "oprt",
    "samsungAr",
    "ucweb",
    "UCShellJava",
    "puffinDevice",
  ]
  let w = window()
  let rec loop = i =>
    if i >= arrayLength(keys) {
      ()
    } else {
      let key = getInt(keys, i)
      let value = get(w, key)
      if isTruthy(value) && isObject(value) {
        arrayPush(flavors, key)->ignore
      }
      loop(i + 1)
    }
  loop(0)
  arraySort(flavors)
}
