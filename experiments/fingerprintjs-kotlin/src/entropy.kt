internal fun isFinalResultLoaded(loadResult: dynamic): Boolean = jsTypeof(loadResult) != "function"

internal fun loadSource(source: dynamic, sourceOptions: dynamic): dynamic {
    val sourceLoadPromise = suppressUnhandledRejectionWarning(
        jsPromise { resolveLoad, _ ->
            val loadStartTime = jsDateNow()
            val bound = jsBind(source, jsNull, sourceOptions)
            awaitIfAsync({ jsCall0(bound, jsUndefined) }) { success, loadArgsValue ->
                val loadDuration = jsDateNow() - loadStartTime
                if (!success) {
                    resolveLoad({
                        val component = obj()
                        jsSet(component, "error", loadArgsValue)
                        jsSet(component, "duration", loadDuration)
                        component
                    })
                    return@awaitIfAsync
                }
                val loadResult = loadArgsValue
                if (isFinalResultLoaded(loadResult)) {
                    resolveLoad({
                        val component = obj()
                        jsSet(component, "value", loadResult)
                        jsSet(component, "duration", loadDuration)
                        component
                    })
                    return@awaitIfAsync
                }
                resolveLoad({
                    jsPromise { resolveGet, _ ->
                        val getStartTime = jsDateNow()
                        awaitIfAsync({ jsCall0(loadResult, jsUndefined) }) { getSuccess, getValue ->
                            val duration = loadDuration + jsDateNow() - getStartTime
                            val component = obj()
                            if (!getSuccess) {
                                jsSet(component, "error", getValue)
                            } else {
                                jsSet(component, "value", getValue)
                            }
                            jsSet(component, "duration", duration)
                            resolveGet(component)
                        }
                    }
                })
            }
        },
    )

    return namedFunction("getComponent") {
        sourceLoadPromise.then({ finalizeSource: dynamic -> jsCall0(finalizeSource, jsUndefined) })
    }
}

internal fun loadSources(
    sources: dynamic,
    sourceOptions: dynamic,
    excludeSources: dynamic,
    loopReleaseInterval: dynamic,
): dynamic {
    val exclude = if (isMissing(excludeSources)) arr() else excludeSources
    val interval = if (isUndefined(loopReleaseInterval)) 16.0 else jsNumber(loopReleaseInterval)
    val allKeys = jsKeys(sources)
    val includedSources = arr()
    val keyCount = jsLength(allKeys)
    var i = 0
    while (i < keyCount) {
        val sourceKey = jsIndex(allKeys, i)
        if (excludes(exclude, sourceKey)) {
            jsPush(includedSources, sourceKey)
        }
        i += 1
    }

    val sourceGettersPromise = suppressUnhandledRejectionWarning(
        mapWithBreaks(includedSources, { sourceKey, _ ->
            loadSource(jsGet(sources, jsString(sourceKey)), sourceOptions)
        }, interval),
    )

    return namedFunction("getComponents") {
        sourceGettersPromise.then({ sourceGetters: dynamic ->
            mapWithBreaks(sourceGetters, { sourceGetter, _ ->
                suppressUnhandledRejectionWarning(jsCall0(sourceGetter, jsUndefined))
            }, interval).then({ componentPromises: dynamic ->
                jsPromiseAll(componentPromises).then({ componentArray: dynamic ->
                    val components = obj()
                    val count = jsLength(includedSources)
                    var index = 0
                    while (index < count) {
                        jsSet(components, jsString(jsIndex(includedSources, index)), jsIndex(componentArray, index))
                        index += 1
                    }
                    components
                })
            })
        })
    }
}

internal fun transformSource(source: dynamic, transformValue: dynamic): dynamic {
    fun transformLoadResult(loadResult: dynamic): dynamic {
        if (isFinalResultLoaded(loadResult)) {
            return jsCall1(transformValue, jsUndefined, loadResult)
        }
        return {
            val getResult = jsCall0(loadResult, jsUndefined)
            if (isPromise(getResult)) {
                getResult.then({ value: dynamic -> jsCall1(transformValue, jsUndefined, value) })
            } else {
                jsCall1(transformValue, jsUndefined, getResult)
            }
        }
    }

    return { options: dynamic ->
        val loadResult = jsCall1(source, jsUndefined, options)
        if (isPromise(loadResult)) {
            loadResult.then({ value: dynamic -> transformLoadResult(value) })
        } else {
            transformLoadResult(loadResult)
        }
    }
}
