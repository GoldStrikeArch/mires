// Browser driving for the checker and the timing tool, after experiments/browser-workloads/tools/
// run.mjs: selenium-webdriver 4.41 from the root node_modules through the shared helper
// experiments/fingerprintjs-rust-compact/tests/webdriver.mjs (imported, not modified): Chrome
// --headless=new, Firefox -headless, Safari headed (it has no headless mode). A browser whose
// WebDriver session cannot start is `unavailable` with the driver's error, never a pass.
//
// Files are served from a loopback server at their repository-relative URLs under a per-load path
// token, /__load/<N>/<repository path>, from an explicit whitelist, with cache-control: no-store;
// relative imports between served files resolve as they do in Node (and keep the token), with no
// import map (Firefox does not apply a page's import map to modules imported from a WebDriver
// script). Every request is attributed to the page load that made it: by the token in its path,
// or by the token in its Referer (an absolute-path image or fetch from the page); each load is
// closed out (about:blank, then a short drain) before the next starts, so a late request is
// charged to its own cell. The pages are served with COOP same-origin and COEP require-corp, so
// performance.now() gets cross-origin-isolated resolution, and with a Content-Security-Policy that
// allows scripts and connections only from this origin: a data: or blob: module or script is
// refused, and the page reports it (`cspViolations`).

import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { extname, relative, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { repositoryRoot } from './registry.mjs'

const CONTENT_TYPES = {
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
}

export const PAGE = '/library-matrix.html'
export const HEARTBEAT = '/__heartbeat'
export const CSP = "default-src 'self'; script-src 'self' 'unsafe-eval'; connect-src 'self'; img-src 'self'; frame-src 'self'; worker-src 'self'; object-src 'none'; base-uri 'none'"

// The page load a request belongs to: the /__load/<N>/ token in its path, else in its Referer.
export function attributeRequest(pathname, referer) {
  const own = /^\/__load\/(\d+)(\/.*)$/.exec(pathname)
  if (own) return { load: own[1], path: own[2], via: 'path' }
  let via = null
  try {
    via = referer ? /^\/__load\/(\d+)\//.exec(new URL(referer).pathname) : null
  } catch {
    via = null
  }
  return { load: via ? via[1] : null, path: pathname, via: via ? 'referer' : 'none' }
}

export async function startServer(files) {
  const served = new Map()
  for (const path of files) served.set(`/${relative(repositoryRoot, path)}`, path)
  const requests = []
  const beats = new Map()
  const server = createServer(async (request, response) => {
    try {
      const url = new URL(request.url, 'http://127.0.0.1')
      if (url.pathname === HEARTBEAT) {
        beats.set(url.searchParams.get('load'), { time: Date.now(), label: url.searchParams.get('label') })
        request.resume()
        response.writeHead(204).end()
        return
      }
      const attributed = attributeRequest(url.pathname, request.headers.referer)
      requests.push({ ...attributed, time: Date.now() })
      if (attributed.path === PAGE) {
        response.writeHead(200, {
          'content-type': 'text/html; charset=utf-8',
          'cache-control': 'no-store',
          'cross-origin-opener-policy': 'same-origin',
          'cross-origin-embedder-policy': 'require-corp',
          'content-security-policy': CSP,
        })
        response.end('<!doctype html><meta charset="utf-8"><title>library matrix</title>')
        return
      }
      const path = attributed.via === 'path' ? served.get(attributed.path) : null
      if (!path) {
        response.writeHead(404).end('not found')
        return
      }
      response.writeHead(200, {
        'content-type': CONTENT_TYPES[extname(path)] || 'application/octet-stream',
        'cache-control': 'no-store',
        'cross-origin-resource-policy': 'same-origin',
      })
      response.end(await readFile(path))
    } catch (error) {
      response.writeHead(500).end(String(error))
    }
  })
  await new Promise((resolvePromise, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolvePromise)
  })
  const origin = `http://127.0.0.1:${server.address().port}`
  return {
    origin,
    // `query` carries ?load=N (and, for a second instance of one module, &instance=b); the load
    // also becomes the path token, so the module's relative imports carry it too.
    url: (path, query = '') => {
      const load = /[?&]load=(\d+)/.exec(query)?.[1] ?? '0'
      return `${origin}/__load/${load}/${relative(repositoryRoot, path)}${query}`
    },
    requests,
    beats,
    servedPath: (path) => `/${relative(repositoryRoot, path)}`,
    unattributed: () => requests.filter((request) => request.load === null),
    close: () => new Promise((resolvePromise) => server.close(() => resolvePromise())),
  }
}

let webdriverHelper
async function helper() {
  webdriverHelper ||= await import(pathToFileURL(resolve(repositoryRoot, 'experiments/fingerprintjs-rust-compact/tests/webdriver.mjs')))
  return webdriverHelper
}

// A WebDriver session and the identity of the browser behind it, or { error } when it cannot start.
export async function openBrowser(name, { scriptTimeoutMs = 600_000 } = {}) {
  const { browserIdentity, createDriver } = await helper()
  let driver
  try {
    driver = await createDriver(name, repositoryRoot)
  } catch (error) {
    return { error: String(error?.message || error).split('\n').slice(0, 6).join('\n') }
  }
  try {
    await driver.manage().setTimeouts({ pageLoad: 60_000, script: scriptTimeoutMs })
    const identity = await browserIdentity(driver)
    const capabilities = await driver.getCapabilities()
    let driverVersion = capabilities.get('chrome')?.chromedriverVersion ?? capabilities.get('moz:geckodriverVersion') ?? null
    if (name === 'safari') {
      const version = spawnSync('safaridriver', ['--version'], { encoding: 'utf8' })
      driverVersion = String(version.stdout || version.stderr || '').trim() || null
    }
    const environment = {
      name: identity.name,
      version: identity.version,
      label: `${identity.name} ${identity.version}`,
      platform: identity.platform,
      headless: name !== 'safari',
      driver: driverVersion,
      userAgent: await driver.executeScript('return navigator.userAgent'),
    }
    return { driver, environment, name, capabilities }
  } catch (error) {
    await driver.quit().catch(() => {})
    return { error: String(error?.message || error) }
  }
}

// Loads a fresh page, imports `moduleUrl` in it and calls its `exportName(config, { heartbeat })`;
// resolves with the result (a JSON-compatible record). The heartbeat posts a beacon to the server
// through the page's original navigator.sendBeacon, captured before the module can replace it.
export async function runInPage(driver, origin, load, moduleUrl, exportName, config) {
  await driver.get(`${origin}/__load/${load}${PAGE}?load=${load}`)
  return driver.executeAsyncScript(`
    const done = arguments[arguments.length - 1]
    const [moduleUrl, exportName, config, heartbeatUrl] = arguments
    const beacon = navigator.sendBeacon.bind(navigator)
    const heartbeat = (label) => { try { beacon(heartbeatUrl + '&label=' + encodeURIComponent(String(label).slice(0, 200))) } catch {} }
    const cspViolations = []
    document.addEventListener('securitypolicyviolation', (event) => {
      cspViolations.push({ directive: event.effectiveDirective || event.violatedDirective, blocked: String(event.blockedURI).slice(0, 120) })
    })
    // Violation events are dispatched asynchronously (Chrome after a module load fails), so the
    // page waits briefly for them before reporting.
    const finish = (result) => setTimeout(() => {
      if (result && typeof result === 'object') result.cspViolations = cspViolations
      done(result)
    }, 250)
    import(moduleUrl)
      .then((module) => module[exportName](config, { heartbeat }))
      .then(finish, (error) => finish({ status: 'error', error: { name: String(error && error.name), message: String(error && error.message || error), stack: String(error && error.stack) } }))
  `, moduleUrl, exportName, config, `${origin}${HEARTBEAT}?load=${load}`)
}

// Ends a session whose page is stuck in a call that never returns. WebDriver commands queue behind
// the pending script, so `quit` alone would wait forever: the browser itself is closed first, from
// outside the page (Chrome: the DevTools `Browser.close` command on the browser target; Firefox:
// SIGKILL to the `moz:processID` the session reports), then the session is quit with a bound.
export async function forceClose(session) {
  try {
    if (session.name === 'chrome') {
      const address = session.capabilities.get('goog:chromeOptions')?.debuggerAddress
      if (address) {
        const version = await (await fetch(`http://${address}/json/version`)).json()
        await new Promise((resolvePromise) => {
          const socket = new WebSocket(version.webSocketDebuggerUrl)
          const done = () => { try { socket.close() } catch {} resolvePromise() }
          socket.onopen = () => socket.send(JSON.stringify({ id: 1, method: 'Browser.close' }))
          socket.onmessage = done
          socket.onerror = done
          socket.onclose = done
          setTimeout(done, 5_000)
        })
      }
    } else if (session.name === 'firefox') {
      const pid = session.capabilities.get('moz:processID')
      if (pid) process.kill(pid, 'SIGKILL')
    }
  } catch {
    // Fall through to the bounded quit.
  }
  await Promise.race([session.driver.quit().catch(() => {}), new Promise((resolvePromise) => setTimeout(resolvePromise, 15_000))])
}

// runInPage under a hang watchdog: when the page sends no heartbeat for `callTimeoutMs` (a call
// that never returns, such as a panic handler that loops), the WebDriver session is quit, which
// ends the page, and the cell resolves as `hang`. The caller then needs a new session
// (`sessionLost: true`). `requests` are the server requests made during the cell.
export async function runWatched(session, server, load, moduleUrl, exportName, config, { callTimeoutMs }) {
  const started = Date.now()
  server.beats.set(String(load), { time: Date.now(), label: 'page load' })
  let timer
  const watchdog = new Promise((resolvePromise) => {
    timer = setInterval(() => {
      const beat = server.beats.get(String(load))
      const silent = Date.now() - beat.time
      if (silent > callTimeoutMs) {
        clearInterval(timer)
        resolvePromise({ status: 'hang', error: { message: `no progress for ${Math.round(silent / 1000)} s (limit ${Math.round(callTimeoutMs / 1000)} s) after: ${beat.label}` }, hang: { lastCall: beat.label } })
      }
    }, 250)
  })
  const work = runInPage(session.driver, server.origin, load, moduleUrl, exportName, config)
    .catch((error) => ({ status: 'error', error: { message: String(error?.message || error) } }))
  const result = await Promise.race([work, watchdog])
  clearInterval(timer)
  // Close out this load before the next one starts: leave the page, let in-flight requests land,
  // then take the requests attributed to this load (and any unattributable one made meanwhile).
  let sessionLost = false
  if (result.status === 'hang') {
    await forceClose(session)
    sessionLost = true
  } else {
    await session.driver.get('about:blank').catch(() => {})
  }
  await new Promise((resolvePromise) => setTimeout(resolvePromise, 250))
  const ended = Date.now()
  const requests = server.requests.filter((request) => request.load === String(load)
    || (request.load === null && request.time >= started && request.time <= ended))
  return { result, requests, sessionLost }
}
