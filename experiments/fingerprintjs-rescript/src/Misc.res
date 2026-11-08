open Bindings

let errorToObject = (error: any) => {
  let result = emptyObject()
  set(result, "name", get(error, "name"))
  set(result, "message", get(error, "message"))
  let stack = get(error, "stack")
  if isUndefined(stack) {
    set(result, "stack", jsUndefined)
  } else {
    set(result, "stack", stringSplit(asString(stack), "\n"))
  }
  let keys = objectKeys(error)
  let rec loop = i =>
    if i >= arrayLength(keys) {
      ()
    } else {
      let key = getInt(keys, i)
      set(result, key, get(error, key))
      loop(i + 1)
    }
  loop(0)
  result
}

let isFunctionNative = (func: 'a) =>
  testRe(newRegExp("^function\\s.*?\\{\\s*\\[native code]\\s*}$", ""), jsString(func))
