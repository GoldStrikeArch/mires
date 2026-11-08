open Bindings
open Data
open Async

let isFinalResultLoaded = (loadResult: any) => !isFunction(loadResult)

let loadSource = (source: any, sourceOptions: any) => {
  let sourceLoadPromise = suppressUnhandledRejectionWarning(
    jsPromise((resolveLoad, _reject) => {
      let loadStartTime = dateNow()
      awaitIfAsync(jsBind1(source, jsNull, sourceOptions), (ok, loadArgs1) => {
        let loadDuration = dateNow() -. loadStartTime
        if !ok {
          apply1(resolveLoad, () => {
            let component = emptyObject()
            set(component, "error", loadArgs1)
            set(component, "duration", loadDuration)
            component
          })
        } else if isFinalResultLoaded(loadArgs1) {
          apply1(resolveLoad, () => {
            let component = emptyObject()
            set(component, "value", loadArgs1)
            set(component, "duration", loadDuration)
            component
          })
        } else {
          apply1(resolveLoad, () =>
            jsPromise((resolveGet, _rejectGet) => {
              let getStartTime = dateNow()
              awaitIfAsync(loadArgs1, (getOk, getArgs1) => {
                let duration = loadDuration +. dateNow() -. getStartTime
                if !getOk {
                  let component = emptyObject()
                  set(component, "error", getArgs1)
                  set(component, "duration", duration)
                  apply1(resolveGet, component)
                } else {
                  let component = emptyObject()
                  set(component, "value", getArgs1)
                  set(component, "duration", duration)
                  apply1(resolveGet, component)
                }
              })
            })
          )
        }
      })
    }),
  )
  () => asPromise(then1(sourceLoadPromise, finalizeSource => apply0(finalizeSource)))
}

let loadSources = (sources: any, sourceOptions: any, excludeSources: any, loopReleaseInterval: any) => {
  let keys = objectKeys(sources)
  let includedSources = arrayFilter(keys, (sourceKey, _) => {
    if isNullish(excludeSources) {
      true
    } else {
      excludes(asArray(excludeSources), sourceKey)
    }
  })
  let sourceGettersPromise = suppressUnhandledRejectionWarning(
    fromPromise(
      mapWithBreaks(
        includedSources,
        (sourceKey, _index) => loadSource(get(sources, sourceKey), sourceOptions),
        loopReleaseInterval,
      ),
    ),
  )
  async () => {
    let sourceGetters = await asPromise(sourceGettersPromise)
    let componentPromises = await mapWithBreaks(
      asArray(sourceGetters),
      (sourceGetter, _index) => suppressUnhandledRejectionWarning(fromPromise(apply0(sourceGetter))),
      loopReleaseInterval,
    )
    let componentArray = await asPromise(jsPromiseAll(asArray(componentPromises)))
    let components = emptyObject()
    let rec loop = index =>
      if index >= arrayLength(includedSources) {
        ()
      } else {
        set(components, getInt(includedSources, index), getInt(asArray(componentArray), index))
        loop(index + 1)
      }
    loop(0)
    components
  }
}

let transformSource = (source: any, transformValue: any) => {
  let transformLoadResult = (loadResult: any): any =>
    if isFinalResultLoaded(loadResult) {
      apply1(transformValue, loadResult)
    } else {
      identity(() => {
        let getResult = apply0(loadResult)
        if isPromise(getResult) {
          identity(then1(getResult, transformValue))
        } else {
          apply1(transformValue, getResult)
        }
      })
    }
  (options): any => {
    let loadResult = apply1(source, options)
    if isPromise(loadResult) {
      identity(then1(loadResult, transformLoadResult))
    } else {
      transformLoadResult(loadResult)
    }
  }
}
