import { awaitIfAsync, isPromise, mapWithBreaks, suppressUnhandledRejectionWarning } from './async.js'
import { excludes } from './data.js'

function isFinalResultLoaded(loadResult) {
  return typeof loadResult !== 'function'
}

export function loadSource(source, sourceOptions) {
  const sourceLoadPromise = suppressUnhandledRejectionWarning(
    new Promise((resolveLoad) => {
      const loadStartTime = Date.now()
      awaitIfAsync(source.bind(null, sourceOptions), (...loadArgs) => {
        const loadDuration = Date.now() - loadStartTime
        if (!loadArgs[0]) {
          return resolveLoad(() => ({ error: loadArgs[1], duration: loadDuration }))
        }
        const loadResult = loadArgs[1]
        if (isFinalResultLoaded(loadResult)) {
          return resolveLoad(() => ({ value: loadResult, duration: loadDuration }))
        }
        resolveLoad(
          () =>
            new Promise((resolveGet) => {
              const getStartTime = Date.now()
              awaitIfAsync(loadResult, (...getArgs) => {
                const duration = loadDuration + Date.now() - getStartTime
                if (!getArgs[0]) {
                  return resolveGet({ error: getArgs[1], duration })
                }
                resolveGet({ value: getArgs[1], duration })
              })
            }),
        )
      })
    }),
  )

  return function getComponent() {
    return sourceLoadPromise.then((finalizeSource) => finalizeSource())
  }
}

export function loadSources(sources, sourceOptions, excludeSources, loopReleaseInterval) {
  const includedSources = Object.keys(sources).filter((sourceKey) => excludes(excludeSources, sourceKey))
  const sourceGettersPromise = suppressUnhandledRejectionWarning(
    mapWithBreaks(includedSources, (sourceKey) => loadSource(sources[sourceKey], sourceOptions), loopReleaseInterval),
  )

  return async function getComponents() {
    const sourceGetters = await sourceGettersPromise
    const componentPromises = await mapWithBreaks(
      sourceGetters,
      (sourceGetter) => suppressUnhandledRejectionWarning(sourceGetter()),
      loopReleaseInterval,
    )
    const componentArray = await Promise.all(componentPromises)
    const components = {}
    for (let index = 0; index < includedSources.length; ++index) {
      components[includedSources[index]] = componentArray[index]
    }
    return components
  }
}

export function transformSource(source, transformValue) {
  const transformLoadResult = (loadResult) => {
    if (isFinalResultLoaded(loadResult)) {
      return transformValue(loadResult)
    }
    return () => {
      const getResult = loadResult()
      if (isPromise(getResult)) {
        return getResult.then(transformValue)
      }
      return transformValue(getResult)
    }
  }

  return (options) => {
    const loadResult = source(options)
    if (isPromise(loadResult)) {
      return loadResult.then(transformLoadResult)
    }
    return transformLoadResult(loadResult)
  }
}
