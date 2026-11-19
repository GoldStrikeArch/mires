package main

import "github.com/gopherjs/gopherjs/js"

// A source is a JavaScript function (options) => value | (() => value), where either stage
// may return a promise. Components are plain objects {value, duration} or {error, duration}.

const defaultLoopReleaseInterval = 16

func isFinalResultLoaded(loadResult *js.Object) bool { return !isFunction(loadResult) }

// loadSource starts loading the source and returns a function that gets its component.
// Loading starts synchronously so that loadSources does not wait for one source before
// starting the next.
func loadSource(source, sourceOptions *js.Object) func() *js.Object {
	var finalizeSource func() *js.Object

	sourceLoadPromise := suppressUnhandledRejectionWarning(newPromise(func(resolveLoad, _ *js.Object) {
		loadStartTime := dateNow()
		bound := source.Call("bind", nil, sourceOptions)

		// awaitIfAsync measures the duration of a synchronous source without other
		// microtasks running in between.
		awaitIfAsync(func() *js.Object { return bound.Invoke() }, func(ok bool, loadResult *js.Object) {
			// The first outcome settles the load, as the first resolveLoad call does in the
			// oracle; a non-conforming thenable can report more than one.
			if finalizeSource != nil {
				return
			}
			loadDuration := dateNow() - loadStartTime

			if !ok { // Source loading failed
				finalizeSource = func() *js.Object { return object("error", loadResult, "duration", loadDuration) }
			} else if isFinalResultLoaded(loadResult) { // Source loaded with the final result
				finalizeSource = func() *js.Object { return object("value", loadResult, "duration", loadDuration) }
			} else { // Source loaded with a "get" stage
				finalizeSource = func() *js.Object {
					return newPromise(func(resolveGet, _ *js.Object) {
						getStartTime := dateNow()
						awaitIfAsync(func() *js.Object { return loadResult.Invoke() }, func(ok bool, getResult *js.Object) {
							duration := loadDuration + dateNow() - getStartTime
							if !ok {
								resolveGet.Invoke(object("error", getResult, "duration", duration))
							} else {
								resolveGet.Invoke(object("value", getResult, "duration", duration))
							}
						})
					})
				}
			}
			resolveLoad.Invoke()
		})
	}))

	return func() *js.Object {
		return sourceLoadPromise.Call("then", func() *js.Object { return finalizeSource() })
	}
}

// loadSources starts loading the given sources and returns a function that collects their
// components. The function can be called before the sources have loaded.
func loadSources(sources, sourceOptions, excludeSources *js.Object, loopReleaseInterval float64) func() *js.Object {
	var includedSources []*js.Object
	sourceKeys := keys(sources)
	for i := 0; i < sourceKeys.Length(); i++ {
		if key := sourceKeys.Index(i); excludes(excludeSources, key) {
			includedSources = append(includedSources, key)
		}
	}

	// mapWithBreaks lets asynchronous sources complete between the synchronous ones, so
	// that their durations are measured correctly.
	sourceGetters, sourceGettersLoaded := mapWithBreaks(includedSources, func(key *js.Object, _ int) func() *js.Object {
		return loadSource(getKey(sources, key), sourceOptions)
	}, loopReleaseInterval)
	suppressUnhandledRejectionWarning(sourceGettersLoaded)

	return func() *js.Object {
		return async(func(resolve, reject *js.Object) {
			await(sourceGettersLoaded, reject, func(*js.Object) {
				componentPromises, gettersStarted := mapWithBreaks(sourceGetters, func(getSource func() *js.Object, _ int) *js.Object {
					return suppressUnhandledRejectionWarning(getSource())
				}, loopReleaseInterval)

				await(gettersStarted, reject, func(*js.Object) {
					await(js.Global.Get("Promise").Call("all", componentPromises), reject, func(componentArray *js.Object) {
						// The component keys keep the order of the source keys.
						components := newObject()
						for index, key := range includedSources {
							setKey(components, key, componentArray.Index(index))
						}
						resolve.Invoke(components)
					})
				})
			})
		})
	}
}

// transformSource modifies a source by transforming its value, keeping the source's
// properties: synchronous or asynchronous, one or two stages.
func transformSource(source, transformValue *js.Object) *js.Object {
	transformLoadResult := func(loadResult *js.Object) *js.Object {
		if isFinalResultLoaded(loadResult) {
			return transformValue.Invoke(loadResult)
		}
		return jsFunc("", 0, func(*js.Object, []*js.Object) any {
			getResult := loadResult.Invoke()
			if isPromise(getResult) {
				return getResult.Call("then", transformValue)
			}
			return transformValue.Invoke(getResult)
		})
	}

	return jsFunc("", 1, func(_ *js.Object, args []*js.Object) any {
		loadResult := source.Invoke(arg(args, 0))
		if isPromise(loadResult) {
			return loadResult.Call("then", transformLoadResult)
		}
		return transformLoadResult(loadResult)
	})
}
