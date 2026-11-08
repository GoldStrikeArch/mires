open Bindings

let isGreaseBrand = (brand: string) => testRe(newRegExp("not", "i"), brand)

let getUserAgentData = () => {
  let uaData = get(navigator(), "userAgentData")
  if isFalsy(uaData) {
    fromPromise(Promise.resolve(jsUndefined))
  } else {
    let brandsRaw = asArray(get(uaData, "brands"))
    let filtered = arrayFilter(brandsRaw, (item, _) => !isGreaseBrand(asString(get(item, "brand"))))
    let mapped = arrayMap(filtered, (item, _) => asString(get(item, "brand")))
    let brands = if arrayLength(mapped) > 1 {
      arrayFilter(mapped, (brand, _) => brand != "Chromium")
    } else {
      mapped
    }
    let result = emptyObject()
    set(result, "brands", brands)
    set(result, "mobile", get(uaData, "mobile"))
    set(result, "platform", get(uaData, "platform"))
    if isTruthy(get(uaData, "getHighEntropyValues")) {
      let highEntropyPromise = call1(
        uaData,
        "getHighEntropyValues",
        ["architecture", "bitness", "model", "platformVersion"],
      )
      then_(
        highEntropyPromise,
        highEntropy => {
          set(result, "architecture", get(highEntropy, "architecture"))
          set(result, "bitness", get(highEntropy, "bitness"))
          set(result, "model", get(highEntropy, "model"))
          set(result, "platformVersion", get(highEntropy, "platformVersion"))
          result
        },
        error =>
          if isDOMException(error) && errorName(error) == "NotAllowedError" {
            set(result, "highEntropyStatus", "not_allowed")
            result
          } else {
            throwJs(error)
          },
      )
    } else {
      fromPromise(Promise.resolve(result))
    }
  }
}
