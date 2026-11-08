open Bindings
open Hashing
open Misc
open Async
open Confidence

let escapeComponentKey = (key: string) =>
  asString(stringReplace(key, newRegExp("([:|\\\\])", "g"), "\\$1"))

let componentsToCanonicalString = (components: any) => {
  let keys = arraySort(objectKeys(components))
  let result = ref("")
  let rec loop = i =>
    if i >= arrayLength(keys) {
      result.contents
    } else {
      let componentKey = getInt(keys, i)
      let component = get(components, componentKey)
      let value = if hasKey(component, "error") {
        "error"
      } else {
        jsString(jsonStringify(get(component, "value")))
      }
      result :=
        result.contents ++
        if result.contents == "" {
          ""
        } else {
          "|"
        } ++
        escapeComponentKey(componentKey) ++
        ":" ++
        value
      loop(i + 1)
    }
  loop(0)
}

let componentsToDebugString = (components: any) =>
  jsonStringifyWith(
    components,
    (_key, value) =>
      if isError(value) {
        errorToObject(value)
      } else {
        value
      },
    2,
  )

let hashComponents = (components: any) => x64hash128(componentsToCanonicalString(components), identity(0))

let makeLazyGetResult = (components: any) => {
  let visitorIdCache = ref(jsUndefined)
  let confidence = getConfidence(components)
  let result = emptyObject()
  let getter = makeGetterUsingThis(thisVal => {
    if isUndefined(visitorIdCache.contents) {
      visitorIdCache := identity(hashComponents(get(thisVal, "components")))
    }
    visitorIdCache.contents
  })
  let setter = visitorId => {
    visitorIdCache := visitorId
  }
  defineProperty(getter, "name", {"value": "get visitorId", "configurable": true})->ignore
  defineProperty(setter, "name", {"value": "set visitorId", "configurable": true})->ignore
  defineProperty(
    result,
    "visitorId",
    {
      "get": getter,
      "set": setter,
      "enumerable": true,
      "configurable": true,
    },
  )->ignore
  set(result, "confidence", confidence)
  set(result, "components", components)
  set(result, "version", algorithmVersion)
  result
}

let prepareForSources = (delayFallback: any) => {
  let delay = if isUndefined(delayFallback) {
    50.
  } else {
    asFloat(delayFallback)
  }
  requestIdleCallbackIfAvailable(delay, delay *. 2.)
}

let makeAgent = (getComponents: unit => promise<any>, debug: any) => {
  let creationTime = dateNow()
  let agent = emptyObject()
  set(agent, "get", (options: any) => {
    let startTime = dateNow()
    then1(fromPromise(getComponents()), components => {
      let result = makeLazyGetResult(components)
      if isTruthy(debug) || (isTruthy(options) && isTruthy(get(options, "debug"))) {
        let n = navigator()
        let message =
          "Copy the text below to get the debug data:\n\n```\nversion: " ++
          asString(get(result, "version")) ++
          "\nuserAgent: " ++
          asString(get(n, "userAgent")) ++
          "\ntimeBetweenLoadAndGet: " ++
          jsString(startTime -. creationTime) ++
          "\nvisitorId: " ++
          asString(get(result, "visitorId")) ++
          "\ncomponents: " ++
          componentsToDebugString(components) ++
          "\n```"
        call1(get(globalThis, "console"), "log", message)->ignore
      }
      result
    })
  })
  agent
}

let monitor: unit => unit = %raw(`
  function() {
    if (window.__fpjs_d_m || Math.random() >= 0.001) {
      return
    }
    try {
      var request = new XMLHttpRequest()
      request.open("get", "https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring", true)
      request.send()
    } catch (error) {
      console.error(error)
    }
  }
`)

let load = (options: any) => {
  let options = if isNullish(options) {
    emptyObject()
  } else {
    options
  }
  let delayFallback = get(options, "delayFallback")
  let debug = get(options, "debug")
  let monitoring = get(options, "monitoring")
  if isUndefined(monitoring) || isTruthy(monitoring) {
    monitor()
  }
  fromPromise(
    {
      let run = async () => {
        let _ = await prepareForSources(delayFallback)
        let sourceOptions = emptyObject()
        set(sourceOptions, "cache", emptyObject())
        set(sourceOptions, "debug", debug)
        let getComponents = Sources.loadBuiltinSources(sourceOptions)
        makeAgent(getComponents, debug)
      }
      run()
    },
  )
}
