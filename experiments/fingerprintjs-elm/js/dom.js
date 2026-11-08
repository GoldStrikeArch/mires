import { wait } from './async.js'
import { parseSimpleCssSelector } from './data.js'

export async function withIframe(action, initialHtml, domPollInterval = 50) {
  const d = document

  while (!d.body) {
    await wait(domPollInterval)
  }

  const iframe = d.createElement('iframe')

  try {
    await new Promise((_resolve, _reject) => {
      let isComplete = false
      const resolve = () => {
        isComplete = true
        _resolve()
      }
      const reject = (error) => {
        isComplete = true
        _reject(error)
      }

      iframe.onload = resolve
      iframe.onerror = reject
      const { style } = iframe
      style.setProperty('display', 'block', 'important')
      style.position = 'absolute'
      style.top = '0'
      style.left = '0'
      style.visibility = 'hidden'
      if (initialHtml && 'srcdoc' in iframe) {
        iframe.srcdoc = initialHtml
      } else {
        iframe.src = 'about:blank'
      }
      d.body.appendChild(iframe)

      const checkReadyState = () => {
        if (isComplete) return
        if (iframe.contentWindow?.document?.readyState === 'complete') {
          resolve()
        } else {
          setTimeout(checkReadyState, 10)
        }
      }
      checkReadyState()
    })

    while (!iframe.contentWindow?.document?.body) {
      await wait(domPollInterval)
    }

    return await action(iframe, iframe.contentWindow)
  } finally {
    iframe.parentNode?.removeChild(iframe)
  }
}

export function selectorToElement(selector) {
  const [tag, attributes] = parseSimpleCssSelector(selector)
  const element = document.createElement(tag ?? 'div')
  for (const name of Object.keys(attributes)) {
    const value = attributes[name].join(' ')
    if (name === 'style') {
      addStyleString(element.style, value)
    } else {
      element.setAttribute(name, value)
    }
  }
  return element
}

export function addStyleString(style, source) {
  for (const property of source.split(';')) {
    const match = /^\s*([\w-]+)\s*:\s*(.+?)(\s*!([\w-]+))?\s*$/.exec(property)
    if (match) {
      const [, name, value, , priority] = match
      style.setProperty(name, value, priority || '')
    }
  }
}

export function isAnyParentCrossOrigin() {
  let currentWindow = window

  for (;;) {
    const parentWindow = currentWindow.parent
    if (!parentWindow || parentWindow === currentWindow) {
      return false
    }

    try {
      if (parentWindow.location.origin !== currentWindow.location.origin) {
        return true
      }
    } catch (error) {
      if (error instanceof Error && error.name === 'SecurityError') {
        return true
      }
      throw error
    }

    currentWindow = parentWindow
  }
}
