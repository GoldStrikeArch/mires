open Bindings

let getMathFingerprint = () => {
  let math = get(globalThis, "Math")
  let fallback = (_value: float) => 0.
  let nativeFn = name => {
    let fn = get(math, name)
    if isFalsy(fn) {
      fallback
    } else {
      value => asFloat(apply1(fn, value))
    }
  }
  let acos = nativeFn("acos")
  let acosh = nativeFn("acosh")
  let asin = nativeFn("asin")
  let asinh = nativeFn("asinh")
  let atanh = nativeFn("atanh")
  let atan = nativeFn("atan")
  let sin = nativeFn("sin")
  let sinh = nativeFn("sinh")
  let cos = nativeFn("cos")
  let cosh = nativeFn("cosh")
  let tan = nativeFn("tan")
  let tanh = nativeFn("tanh")
  let exp = nativeFn("exp")
  let expm1 = nativeFn("expm1")
  let log1p = nativeFn("log1p")
  let powPI = value => mathPow(mathPI, value)
  let acoshPf = value => mathLog(value +. mathSqrt(value *. value -. 1.))
  let asinhPf = value => mathLog(value +. mathSqrt(value *. value +. 1.))
  let atanhPf = value => mathLog((1. +. value) /. (1. -. value)) /. 2.
  let sinhPf = value => mathExp(value) -. 1. /. mathExp(value) /. 2.
  let coshPf = value => (mathExp(value) +. 1. /. mathExp(value)) /. 2.
  let expm1Pf = value => mathExp(value) -. 1.
  let tanhPf = value => (mathExp(2. *. value) -. 1.) /. (mathExp(2. *. value) +. 1.)
  let log1pPf = value => mathLog(1. +. value)
  let result = emptyObject()
  set(result, "acos", acos(0.123124234234234242))
  set(result, "acosh", acosh(1e308))
  set(result, "acoshPf", acoshPf(1e154))
  set(result, "asin", asin(0.123124234234234242))
  set(result, "asinh", asinh(1.))
  set(result, "asinhPf", asinhPf(1.))
  set(result, "atanh", atanh(0.5))
  set(result, "atanhPf", atanhPf(0.5))
  set(result, "atan", atan(0.5))
  set(result, "sin", sin(-1e300))
  set(result, "sinh", sinh(1.))
  set(result, "sinhPf", sinhPf(1.))
  set(result, "cos", cos(10.000000000123))
  set(result, "cosh", cosh(1.))
  set(result, "coshPf", coshPf(1.))
  set(result, "tan", tan(-1e300))
  set(result, "tanh", tanh(1.))
  set(result, "tanhPf", tanhPf(1.))
  set(result, "exp", exp(1.))
  set(result, "expm1", expm1(1.))
  set(result, "expm1Pf", expm1Pf(1.))
  set(result, "log1p", log1p(10.))
  set(result, "log1pPf", log1pPf(10.))
  set(result, "powPI", powPI(-100.))
  result
}
