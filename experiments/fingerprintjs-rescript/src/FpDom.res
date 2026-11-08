open Bindings
open Data
open Async

let addStyleString = (style: any, source: string) => {
  let properties = stringSplit(source, ";")
  let rec loop = i =>
    if i >= arrayLength(properties) {
      ()
    } else {
      let property = getInt(properties, i)
      let match = execRe(
        newRegExp("^\\s*([\\w-]+)\\s*:\\s*(.+?)(\\s*!([\\w-]+))?\\s*$", ""),
        property,
      )
      if isTruthy(match) {
        let name = asString(getInt(match, 1))
        let value = asString(getInt(match, 2))
        let priority = orElse(getInt(match, 4), "")
        setProperty(style, name, value, asString(priority))
      }
      loop(i + 1)
    }
  loop(0)
}

let selectorToElement = (selector: string) => {
  let (tag, attributes) = parseSimpleCssSelector(selector)
  let element = createElement(document(), if isUndefined(tag) {
    "div"
  } else {
    asString(tag)
  })
  let names = objectKeys(attributes)
  let rec loop = i =>
    if i >= arrayLength(names) {
      ()
    } else {
      let name = getInt(names, i)
      let value = arrayJoin(asArray(get(attributes, name)), " ")
      if name == "style" {
        addStyleString(get(element, "style"), value)
      } else {
        setAttribute(element, name, value)
      }
      loop(i + 1)
    }
  loop(0)
  element
}

let isAnyParentCrossOrigin = () => {
  let rec loop = (currentWindow: any) => {
    let parentWindow = get(currentWindow, "parent")
    if isFalsy(parentWindow) || strictEq(parentWindow, currentWindow) {
      false
    } else {
      let outcome = runAction(() =>
        !strictEq(get(get(parentWindow, "location"), "origin"), get(get(currentWindow, "location"), "origin"))
      )
      if asBool(getInt(outcome, 0)) {
        if asBool(getInt(outcome, 1)) {
          true
        } else {
          loop(parentWindow)
        }
      } else {
        let error = getInt(outcome, 1)
        if isError(error) && errorName(error) == "SecurityError" {
          true
        } else {
          throwJs(error)
        }
      }
    }
  }
  loop(window())
}

let withIframe = (action: 'action, initialHtml: any, pollInterval: any) => {
  let poll = if isUndefined(pollInterval) {
    50
  } else {
    asInt(pollInterval)
  }
  let d = document()
  let rec waitForBody = async () =>
    if isFalsy(get(d, "body")) {
      let _ = await wait(poll, jsUndefined)
      await waitForBody()
    } else {
      ()
    }
  let run = async () => {
    await waitForBody()
    let iframe = createElement(d, "iframe")
    let cleanup = () => {
      let parent = get(iframe, "parentNode")
      if isTruthy(parent) {
        removeChild(parent, iframe)->ignore
      }
    }
    let loadOutcome = runAction(() => {
      let p = jsPromise((_resolve, _reject) => {
        let isComplete = ref(false)
        let resolve = () =>
          if !isComplete.contents {
            isComplete := true
            apply1(_resolve, jsUndefined)->ignore
          }
        let reject = error =>
          if !isComplete.contents {
            isComplete := true
            apply1(_reject, error)->ignore
          }
        set(iframe, "onload", resolve)
        set(iframe, "onerror", reject)
        let style = get(iframe, "style")
        setProperty(style, "display", "block", "important")
        set(style, "position", "absolute")
        set(style, "top", "0")
        set(style, "left", "0")
        set(style, "visibility", "hidden")
        if isTruthy(initialHtml) && hasKey(iframe, "srcdoc") {
          set(iframe, "srcdoc", initialHtml)
        } else {
          set(iframe, "src", "about:blank")
        }
        appendChild(get(d, "body"), iframe)->ignore
        let rec checkReadyState = () =>
          if isComplete.contents {
            ()
          } else {
            let contentWindow = get(iframe, "contentWindow")
            let doc = if isTruthy(contentWindow) {
              get(contentWindow, "document")
            } else {
              jsUndefined
            }
            if isTruthy(doc) && asString(get(doc, "readyState")) == "complete" {
              resolve()
            } else {
              setTimeout(checkReadyState, 10)->ignore
            }
          }
        checkReadyState()
      })
      p
    })
    let body = async () => {
      if !asBool(getInt(loadOutcome, 0)) {
        throwJs(getInt(loadOutcome, 1))
      }
      let _ = await asPromise(getInt(loadOutcome, 1))
      let rec waitForIframeBody = async () => {
        let contentWindow = get(iframe, "contentWindow")
        let doc = if isTruthy(contentWindow) {
          get(contentWindow, "document")
        } else {
          jsUndefined
        }
        let body = if isTruthy(doc) {
          get(doc, "body")
        } else {
          jsUndefined
        }
        if isFalsy(body) {
          let _ = await wait(poll, jsUndefined)
          await waitForIframeBody()
        } else {
          ()
        }
      }
      await waitForIframeBody()
      let result = apply2(action, iframe, get(iframe, "contentWindow"))
      if isPromise(result) {
        await asPromise(result)
      } else {
        result
      }
    }
    await asPromise(tryFinallyAsync(body, cleanup))
  }
  fromPromise(run())
}
