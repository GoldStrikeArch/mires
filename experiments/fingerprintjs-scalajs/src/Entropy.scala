package fp

import scala.scalajs.js

object Entropy {
  def loadSource(source: js.Function, sourceOptions: js.Any): js.Function0[js.Promise[js.Dynamic]] = {
    val sourceLoadPromise = Async.suppressUnhandledRejectionWarning(
      new js.Promise[js.Function0[js.Any]]((resolveLoad, _) => {
        val loadStartTime = Host.dateNow()
        val bound: js.Function0[js.Any] = () =>
          Host.dyn(source).call(null, sourceOptions).asInstanceOf[js.Any]
        Async.awaitIfAsync(
          bound,
          (ok, result) => {
            val loadDuration = Host.dateNow() - loadStartTime
            if (!ok) {
              resolveLoad(() => js.Dynamic.literal(error = result, duration = loadDuration))
            } else if (!Host.isJsFunction(result)) {
              resolveLoad(() => js.Dynamic.literal(value = result, duration = loadDuration))
            } else {
              resolveLoad(() =>
                new js.Promise[js.Dynamic]((resolveGet, _) => {
                  val getStartTime = Host.dateNow()
                  val getBound: js.Function0[js.Any] = () => Host.dyn(result).apply().asInstanceOf[js.Any]
                  Async.awaitIfAsync(
                    getBound,
                    (getOk, getResult) => {
                      val duration = loadDuration + Host.dateNow() - getStartTime
                      if (!getOk) resolveGet(js.Dynamic.literal(error = getResult, duration = duration))
                      else resolveGet(js.Dynamic.literal(value = getResult, duration = duration))
                    },
                  )
                }),
              )
            }
          },
        )
      }),
    )

    () =>
      sourceLoadPromise.`then`[js.Any]((finalize: js.Function0[js.Any]) => {
        val out = finalize()
        if (Async.isPromise(out)) out
        else js.Promise.resolve(out)
      }).asInstanceOf[js.Promise[js.Dynamic]]
  }

  def loadSources(
      sources: js.Dictionary[js.Function],
      sourceOptions: js.Any,
      excludeSources: js.Array[js.Any],
      loopReleaseInterval: js.UndefOr[Double] = js.undefined,
  ): js.Function0[js.Promise[js.Dynamic]] = {
    val interval = loopReleaseInterval.getOrElse(16.0)
    val included = Host.objectKeys(sources).filter(key => Data.excludes(excludeSources, key))
    val sourceGettersPromise = Async.suppressUnhandledRejectionWarning(
      Async.mapWithBreaks[String, js.Function0[js.Promise[js.Dynamic]]](
        included,
        (sourceKey, _) => loadSource(sources(sourceKey), sourceOptions),
        interval,
      ),
    )

    () =>
      sourceGettersPromise
        .`then`[js.Any] { (sourceGetters: js.Array[js.Function0[js.Promise[js.Dynamic]]]) =>
          Async.mapWithBreaks[js.Function0[js.Promise[js.Dynamic]], js.Promise[js.Dynamic]](
            sourceGetters,
            (sourceGetter, _) => Async.suppressUnhandledRejectionWarning(sourceGetter()),
            interval,
          )
        }
        .`then`[js.Any] { (componentPromises: js.Any) =>
          Host.g.Promise.all(componentPromises).asInstanceOf[js.Promise[js.Any]]
        }
        .`then`[js.Any] { (componentArray: js.Any) =>
          val values = componentArray.asInstanceOf[js.Array[js.Dynamic]]
          val components = js.Dictionary.empty[js.Dynamic]
          var index = 0
          while (index < included.length) {
            components(included(index)) = values(index)
            index += 1
          }
          components.asInstanceOf[js.Any]
        }
        .asInstanceOf[js.Promise[js.Dynamic]]
  }

  def transformSource(source: js.Function, transformValue: js.Function1[js.Any, js.Any]): js.Function = {
    def transformLoadResult(loadResult: js.Any): js.Any =
      if (!Host.isJsFunction(loadResult)) transformValue(loadResult)
      else {
        val getter: js.Function0[js.Any] = () => {
          val getResult = Host.dyn(loadResult).apply().asInstanceOf[js.Any]
          if (Async.isPromise(getResult))
            getResult.asInstanceOf[js.Dynamic].`then`(transformValue).asInstanceOf[js.Any]
          else transformValue(getResult)
        }
        getter
      }

    val wrapped: js.Function1[js.Any, js.Any] = (options: js.Any) => {
      val loadResult = Host.dyn(source).apply(options).asInstanceOf[js.Any]
      if (Async.isPromise(loadResult))
        loadResult.asInstanceOf[js.Dynamic].`then`((value: js.Any) => transformLoadResult(value)).asInstanceOf[js.Any]
      else transformLoadResult(loadResult)
    }
    wrapped
  }

  val loadSourcesFn: js.Function4[js.Dictionary[js.Function], js.Any, js.Array[js.Any], js.UndefOr[Double], js.Function0[js.Promise[js.Dynamic]]] =
    (sources, sourceOptions, excludeSources, loopReleaseInterval) =>
      loadSources(sources, sourceOptions, excludeSources, loopReleaseInterval)

  val transformSourceFn: js.Function2[js.Function, js.Function1[js.Any, js.Any], js.Function] =
    (source, transformValue) => transformSource(source, transformValue)
}
