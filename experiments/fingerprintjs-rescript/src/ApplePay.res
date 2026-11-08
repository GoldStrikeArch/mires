open Bindings
open FpDom
open Async

let getStateFromError = (error: any) => {
  if (
    isError(error) &&
      errorName(error) == "InvalidAccessError" &&
      testRe(newRegExp("\\bfrom\\b.*\\binsecure\\b", "i"), errorMessage(error))
  ) {
    -2
  } else {
    throwJs(error)
  }
}

let getApplePayState = () => {
  let applePaySession = get(window(), "ApplePaySession")
  if isNullish(applePaySession) || !isFunction(get(applePaySession, "canMakePayments")) {
    -1
  } else if isAnyParentCrossOrigin() {
    -3
  } else {
    let outcome = runAction(() => call0(applePaySession, "canMakePayments"))
    if asBool(getInt(outcome, 0)) {
      if asBool(getInt(outcome, 1)) {
        1
      } else {
        0
      }
    } else {
      getStateFromError(getInt(outcome, 1))
    }
  }
}
