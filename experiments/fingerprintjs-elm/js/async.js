export function wait(durationMs, resolveWith) {
  return new Promise((resolve) => setTimeout(resolve, durationMs, resolveWith))
}

function releaseEventLoop() {
  return new Promise((resolve) => {
    const channel = new MessageChannel()
    channel.port1.onmessage = () => resolve()
    channel.port2.postMessage(null)
  })
}

export function requestIdleCallbackIfAvailable(fallbackTimeout, deadlineTimeout = Infinity) {
  const { requestIdleCallback } = window
  if (requestIdleCallback) {
    return new Promise((resolve) => requestIdleCallback.call(window, () => resolve(), { timeout: deadlineTimeout }))
  }
  return wait(Math.min(fallbackTimeout, deadlineTimeout))
}

export function isPromise(value) {
  return !!value && typeof value.then === 'function'
}

export function awaitIfAsync(action, callback) {
  try {
    const returnedValue = action()
    if (isPromise(returnedValue)) {
      returnedValue.then(
        (result) => callback(true, result),
        (error) => callback(false, error),
      )
    } else {
      callback(true, returnedValue)
    }
  } catch (error) {
    callback(false, error)
  }
}

export async function mapWithBreaks(items, callback, loopReleaseInterval = 16) {
  const results = Array(items.length)
  let lastLoopReleaseTime = Date.now()

  for (let i = 0; i < items.length; ++i) {
    results[i] = callback(items[i], i)

    const now = Date.now()
    if (now >= lastLoopReleaseTime + loopReleaseInterval) {
      lastLoopReleaseTime = now
      await releaseEventLoop()
    }
  }

  return results
}

export function suppressUnhandledRejectionWarning(promise) {
  promise.then(undefined, () => undefined)
  return promise
}
