open Bindings
open Async

let areCookiesEnabled = () => {
  let d = document()
  let outcome = runAction(() => {
    set(d, "cookie", "cookietest=1; SameSite=Strict;")
    let result = stringIndexOf(asString(get(d, "cookie")), "cookietest=") != -1
    set(d, "cookie", "cookietest=1; SameSite=Strict; expires=Thu, 01-Jan-1970 00:00:01 GMT")
    result
  })
  if asBool(getInt(outcome, 0)) {
    getInt(outcome, 1)
  } else {
    false
  }
}
