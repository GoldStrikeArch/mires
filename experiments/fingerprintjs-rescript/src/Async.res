open Bindings

let wait = (durationMs: int, resolveWith: 'a): promise<'a> =>
  asPromise(
    jsPromise((resolve, _reject) => {
      setTimeout(() => apply1(resolve, resolveWith), durationMs)->ignore
    }),
  )

let releaseEventLoop = (): promise<unit> =>
  asPromise(
    jsPromise((resolve, _reject) => {
      let channel = newMessageChannel()
      set(get(channel, "port1"), "onmessage", () => apply1(resolve, jsUndefined))
      call1(get(channel, "port2"), "postMessage", jsNull)->ignore
    }),
  )

let requestIdleCallbackIfAvailable = (fallbackTimeout: float, deadlineTimeout: float): promise<
  any,
> => {
  let w = window()
  let ric = get(w, "requestIdleCallback")
  if isTruthy(ric) {
    asPromise(
      jsPromise((resolve, _reject) => {
        jsCall2(ric, w, () => apply1(resolve, jsUndefined), {"timeout": deadlineTimeout})->ignore
      }),
    )
  } else {
    wait(asInt(mathMin(fallbackTimeout, deadlineTimeout)), jsUndefined)
  }
}

let isPromise = (value: 'a) => isTruthy(value) && isFunction(get(value, "then"))

let runAction: 'fn => any = %raw(`function(action) {
  try { return [true, action()] }
  catch (error) { return [false, error] }
}`)

let awaitIfAsync = (action: 'fn, callback: (bool, any) => 'r) => {
  let outcome = runAction(action)
  if asBool(getInt(outcome, 0)) {
    let returnedValue = getInt(outcome, 1)
    if isPromise(returnedValue) {
      then_(
        returnedValue,
        result => callback(true, result),
        error => callback(false, error),
      )->ignore
    } else {
      callback(true, returnedValue)->ignore
    }
  } else {
    callback(false, getInt(outcome, 1))->ignore
  }
}

let mapWithBreaks = async (items: array<'a>, callback: ('a, int) => 'b, loopReleaseInterval: any) => {
  let interval = if isUndefined(loopReleaseInterval) {
    16.
  } else {
    asFloat(loopReleaseInterval)
  }
  let length = arrayLength(items)
  let results = newArray(length)
  let lastLoopReleaseTime = ref(dateNow())
  let rec loop = async i =>
    if i >= length {
      results
    } else {
      jsSetIndex(results, i, callback(getInt(items, i), i))
      let now = dateNow()
      if now >= lastLoopReleaseTime.contents +. interval {
        lastLoopReleaseTime := now
        await releaseEventLoop()
      }
      await loop(i + 1)
    }
  await loop(0)
}

let suppressUnhandledRejectionWarning = (promise: 'a) => {
  then_(promise, jsUndefined, () => jsUndefined)->ignore
  promise
}
