open Bindings
open Browser

let getLanguages = () => {
  let n = navigator()
  let result: array<any> = []
  let language = orElse(
    get(n, "language"),
    orElse(get(n, "userLanguage"), orElse(get(n, "browserLanguage"), get(n, "systemLanguage"))),
  )
  if !isUndefined(language) {
    arrayPush(result, identity([language]))->ignore
  }
  let languages = get(n, "languages")
  if isJsArray(languages) {
    if !(isChromium() && isChromium86OrNewer()) {
      arrayPush(result, identity(asArray(languages)))->ignore
    }
  } else if isString(languages) {
    if isTruthy(languages) {
      arrayPush(result, identity(stringSplit(asString(languages), ",")))->ignore
    }
  }
  result
}
