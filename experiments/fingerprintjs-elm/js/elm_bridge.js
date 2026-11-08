import Elm from '../build/elm.mjs'

let app
let lastResponse

function getApp() {
  if (!app) {
    app = Elm.Fingerprint.init()
    app.ports.response.subscribe((value) => {
      lastResponse = value
    })
  }
  return app
}

export function utf16Units(input) {
  const text = input == null ? '' : String(input)
  const units = new Array(text.length)
  for (let i = 0; i < text.length; i++) units[i] = text.charCodeAt(i)
  return units
}

export function elmCall(payload) {
  lastResponse = undefined
  getApp().ports.request.send(payload)
  if (lastResponse === undefined) {
    throw new Error('Elm port did not respond synchronously')
  }
  if (lastResponse && lastResponse.error) {
    throw new Error(lastResponse.error)
  }
  return lastResponse.result
}

export function elmHash(input, seed) {
  return elmCall({ op: 'hash', units: utf16Units(input), seed: (seed || 0) | 0 })
}
