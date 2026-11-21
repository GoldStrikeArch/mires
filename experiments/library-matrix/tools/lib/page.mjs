// The page-realm bootstrap of a browser cell (README "Checker"). tools/lib/browser.mjs `runInPage`
// inserts it into the freshly loaded page as <script type="module">, with the cell's request as JSON
// text in the element's data-library-matrix attribute, and waits for its result event. The page
// itself then imports the cell runner (check-runner.mjs or protocol.mjs), which imports the
// reference and the port, so all of them run in the page's own realm: globalThis === window, and
// host objects (errors from fetch, URL or Cache, IndexedDB values, structuredClone results,
// TextEncoder output) share the code's own Error, Object and Uint8Array. A module imported from a
// WebDriver script does not in Firefox, where Marionette evaluates the script in a sandbox whose
// global is not the window.
//
// This module has no imports, so its body runs before any other module of the cell: it removes the
// request attribute (with the per-load token) from the DOM and captures the reporting builtins
// (navigator.sendBeacon, JSON.stringify, CustomEvent, EventTarget.prototype.dispatchEvent,
// encodeURIComponent, String) before the runner and port are imported, so later code cannot read
// the token from the DOM or break/intercept reporting by patching those builtins.

const stringify = JSON.stringify
const EventCtor = CustomEvent
const dispatch = EventTarget.prototype.dispatchEvent.bind(document)
const encode = encodeURIComponent
const toString = String
const slice = String.prototype.slice
const beacon = navigator.sendBeacon.bind(navigator)

const element = document.querySelector('script[data-library-matrix]')
const { moduleUrl, exportName, config, heartbeatUrl, token } = JSON.parse(element.dataset.libraryMatrix)
element.removeAttribute('data-library-matrix')

const heartbeat = (label) => {
  try {
    beacon(`${heartbeatUrl}&label=${encode(slice.call(toString(label), 0, 200))}`)
  } catch {
    // A lost heartbeat only brings the watchdog closer.
  }
}

// The result travels as JSON text, so the WebDriver side never holds an object of the page.
const report = (result) => dispatch(new EventCtor('library-matrix-result', { detail: stringify({ token, result }) }))

try {
  const runner = await import(moduleUrl)
  report(await runner[exportName](config, { heartbeat }))
} catch (error) {
  report({ status: 'error', error: { name: toString(error?.name), message: toString(error?.message || error), stack: toString(error?.stack) } })
}
