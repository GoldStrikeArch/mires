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
// This module has no imports, so its body runs before any other module of the cell: the heartbeat
// binds the page's own navigator.sendBeacon before the port can replace it, and the request, with
// the result token, leaves the DOM before the runner is imported, so nothing loaded later can read
// the token or report a result in the runner's place.

const element = document.querySelector('script[data-library-matrix]')
const { moduleUrl, exportName, config, heartbeatUrl, token } = JSON.parse(element.dataset.libraryMatrix)
element.removeAttribute('data-library-matrix')

const beacon = navigator.sendBeacon.bind(navigator)
const heartbeat = (label) => {
  try {
    beacon(`${heartbeatUrl}&label=${encodeURIComponent(String(label).slice(0, 200))}`)
  } catch {
    // A lost heartbeat only brings the watchdog closer.
  }
}

// The result travels as JSON text, so the WebDriver side never holds an object of the page.
const report = (result) => document.dispatchEvent(new CustomEvent('library-matrix-result', { detail: JSON.stringify({ token, result }) }))

try {
  const runner = await import(moduleUrl)
  report(await runner[exportName](config, { heartbeat }))
} catch (error) {
  report({ status: 'error', error: { name: String(error?.name), message: String(error?.message || error), stack: String(error?.stack) } })
}
