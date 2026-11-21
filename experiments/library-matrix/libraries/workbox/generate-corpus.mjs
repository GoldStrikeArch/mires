// Writes corpus.json: one script of Workbox API calls per case (CONTRACT.md "Steps").
// Deterministic: the only inputs are the two fixture files, hashed for the integrity cases.
import {createHash} from 'node:crypto'
import {readFileSync, writeFileSync} from 'node:fs'

const sri = name => 'sha256-' + createHash('sha256').update(readFileSync(new URL(name + '.mjs', import.meta.url))).digest('base64')
const ALPHA = sri('alpha')
const BETA = sri('beta')

// Steps.
const network = source => ({op: 'network', source})
const controller = (fields = {}, id = 'sw') => ({op: 'controller', id, cacheName: 'precache', ...fields})
const add = (entries, id = 'sw') => ({op: 'addToCacheList', controller: id, entries})
const addPlugins = (plugins, id = 'sw') => ({op: 'addPlugins', controller: id, plugins})
const install = (id = 'sw') => ({op: 'install', controller: id})
const activate = (id = 'sw') => ({op: 'activate', controller: id})
const keys = (id = 'sw') => ({op: 'getURLsToCacheKeys', controller: id})
const cachedURLs = (id = 'sw') => ({op: 'getCachedURLs', controller: id})
const keyFor = (url, id = 'sw') => ({op: 'getCacheKeyForURL', controller: id, url})
const matchPrecache = (url, fields = {}, id = 'sw') => ({op: 'matchPrecache', controller: id, url, ...fields})
const router = (id = 'r') => ({op: 'router', id})
const precacheRoute = (options, fields = {}) => ({op: 'precacheRoute', router: 'r', controller: 'sw', ...(options ? {options} : {}), ...fields})
const route = (prefix, handler, fields = {}) => ({op: 'route', router: 'r', prefix, handler, ...fields})
const defaultHandler = (handler, fields = {}) => ({op: 'defaultHandler', router: 'r', handler, ...fields})
const catchHandler = (handler, fields = {}) => ({op: 'catchHandler', router: 'r', handler, ...fields})
const unregister = (routeId, routerId = 'r') => ({op: 'unregisterRoute', router: routerId, route: routeId})
const get = (url, fields = {}) => ({op: 'fetch', router: 'r', url, ...fields})
const dump = {op: 'caches'}

// Handlers and plugins.
const strategy = (name, plugins, cacheName = 'runtime') => ({strategy: name, cacheName, ...(plugins ? {plugins} : {})})
const respond = (text, status = 200) => ({respond: text, status})
const trace = label => ({kind: 'trace', label})
const keep = {kind: 'keep'}
const drop = {kind: 'drop'}
const fail = on => ({kind: 'fail', on})
// Throws the string itself: a non-Error, which Workbox's instanceof Error guards treat apart.
const failValue = on => ({kind: 'fail', on, value: 'not an error'})
const fallback = (url, id = 'sw') => ({kind: 'fallback', url, controller: id})

// A service worker version: a controller, its manifest, a router with the precache route first and
// one runtime route per strategy.
const app = (entries, {plugins, runtime} = {}) => [
  controller(plugins ? {plugins} : {}), add(entries), router(), precacheRoute(),
  route('first/', strategy('CacheFirst', runtime)), route('only/', strategy('CacheOnly', runtime)), route('network/', strategy('NetworkOnly', runtime)),
]

const site = [{url: 'docs/index.html', revision: '1'}, {url: 'guide.html', revision: 'old'}, {url: 'a?lang=en', revision: 'x / 2'}]
const cases = []
const add_case = (id, note, ...steps) => cases.push({id, note, input: {steps: steps.flat()}})

// Cache keys and addToCacheList.
add_case('keys-revisioned', 'revisioned entries get __WB_REVISION__ appended through URLSearchParams (spaces, slashes, an existing query)',
  controller(), add(site), keys(), keyFor('guide.html'), keyFor('a?lang=en'), keyFor('docs/'))
add_case('keys-revisionless', 'null, absent and empty revisions and string entries are their own cache keys',
  controller(), add([{url: 'bundle.218.js', revision: null}, {url: 'app.js'}, 'string-entry.css', {url: 'empty.js', revision: ''}]), keys())
add_case('keys-numeric-revision', 'numeric revisions: 0 is falsy, the rest print with JavaScript Number-to-String, past int32 and past 2^53',
  controller(), add([{url: 'zero.js', revision: 0}, {url: 'seven.js', revision: 7}, {url: 'half.js', revision: 1.5}, {url: 'int32.js', revision: 2 ** 31},
    {url: 'safe.js', revision: 2 ** 53 + 2}, {url: 'huge.js', revision: 1e21}, {url: 'negative.js', revision: -1}, {url: 'tiny.js', revision: 1e-7}]), keys())
add_case('keys-unicode-revision', 'non-ASCII, astral and reserved characters in revisions and paths are percent-encoded by the host URL',
  controller(), add([{url: 'ü/😀.html', revision: 'ü😀 &=#?+'}, {url: 'p%20q.html', revision: '%41'}]), keys(), keyFor('ü/😀.html'), keyFor('%C3%BC/%F0%9F%98%80.html'))
add_case('keys-url-forms', 'hashes, queries, an existing revision parameter, padding, dot segments, cross-origin and non-http entries',
  controller(), add(['a.html#frag', {url: 'q.html?b=2&a=1', revision: '3'}, {url: 'r.html?__WB_REVISION__=9&x', revision: '1'}, ' padded.html ',
    {url: 'dir/./sub/../x.html', revision: '2'}, 'http://x/', {url: 'https://example.com/a?b', revision: '4'}, 'data:,x', 'dir/']),
  keys(), keyFor('a.html'), keyFor('a.html#frag'), keyFor('dir/x.html'), keyFor('http://x'), keyFor('data:,x'))
add_case('keys-duplicate', 'repeated entries with the same key are accepted and keep their first position',
  controller(), add([site[0], 'z.css', site[0], 'z.css']), add([site[1], site[0]]), keys(), cachedURLs())
add_case('keys-conflict', 'a conflicting revision throws after the earlier entries were added; later entries are not',
  controller(), add([{url: 'a.html', revision: '1'}, 'b.html', {url: 'a.html', revision: '2'}, 'c.html']), keys())
add_case('keys-conflict-revisionless', 'a revisionless and a revisioned entry for one URL conflict, in both orders',
  controller(), add([{url: 'a.html', revision: null}, {url: 'a.html', revision: '1'}]), add([{url: 'b.html', revision: '1'}]), add(['b.html']), keys())
add_case('keys-integrity-conflict', 'conflicting integrities for one cache key throw; a conflicting revision is reported before the integrity',
  controller(), add([{url: 'a.html', revision: '1', integrity: 'sha256-x'}, {url: 'a.html', revision: '1', integrity: 'sha256-x'}]),
  add([{url: 'a.html', revision: '1', integrity: 'sha256-y'}]), add([{url: 'a.html', revision: '2', integrity: 'sha256-z'}]), keys())
add_case('keys-unexpected-type', 'null entries and entries without a URL throw add-to-cache-list-unexpected-type with the entry in the message',
  controller(), add([null]), add([{revision: '1'}]), add([{url: '', revision: '1'}]), add([{url: 'ok.html', revision: 'é'}, {revision: 'ü', integrity: 'i'}]), keys())
add_case('keys-invalid-url', 'URLs the host cannot parse throw a TypeError from addToCacheList and getCacheKeyForURL',
  controller(), add(['fine.html', {url: 'http://[', revision: '1'}, 'never.html']), keyFor('http://['), keyFor('fine.html'), keys())
add_case('keys-lookup', 'getCacheKeyForURL resolves against the location and looks up the exact URL, without route variations',
  controller(), add(site), add([{url: 'later.html', revision: '5'}]), keyFor('guide.html'), keyFor('guide'), keyFor('guide.html?utm_source=x'),
  keyFor('./docs/index.html'), keyFor('unknown.html'), keys())

// install and activate.
add_case('install-fresh', 'install fetches every entry once; a second install finds them all cached',
  controller(), add(site), install(), install(), dump)
add_case('install-upgrade', 'a new version with a changed, a removed and a new entry; activate deletes outdated keys',
  controller(), add(site), install(), activate(), network('beta'),
  controller({}, 'v2'), add([site[0], {url: 'guide.html', revision: 'new'}, {url: 'new.html', revision: '1'}], 'v2'), install('v2'), activate('v2'), activate('v2'),
  router(), precacheRoute(undefined, {controller: 'v2'}), network('offline'), get('guide.html'), get('docs/'), get('a?lang=en'), get('new'))
add_case('install-empty', 'an empty precache installs nothing and opens no cache; activate opens (creates) the precache',
  controller(), install(), dump, activate(), dump)
add_case('install-missing', 'a 404 fails install with bad-precaching-response after the earlier entry was cached',
  controller({plugins: [trace('pre')]}), add(['kept.html']), install(), add([{url: 'gone.html', revision: '1'}]), network('missing'), install(), activate())
add_case('install-offline', 'a requestWillFetch failure becomes plugin-error-request-will-fetch',
  controller({plugins: [trace('pre')]}), add([{url: 'guide.html', revision: '1'}]), network('offline'), install())
add_case('install-broken', 'a fetch that rejects runs fetchDidFail and fails install with the host TypeError',
  controller({plugins: [trace('pre')]}), add([{url: 'guide.html', revision: '1'}]), network('broken'), install())
add_case('install-integrity', 'manifest integrity reaches the fetch: the matching hash installs, the other fails',
  controller(), add([{url: 'good.html', revision: '1', integrity: ALPHA}]), install(), add([{url: 'bad.html', revision: '1', integrity: BETA}]), install(), keys())
add_case('install-keep-plugin', 'a custom cacheWillUpdate replaces the default cacheability check, so a 404 is precached',
  controller({plugins: [keep]}), add([{url: 'guide.html', revision: '1'}]), network('missing'), install())
add_case('install-drop-plugin', 'a cacheWillUpdate that refuses every response fails install with status 200',
  controller({plugins: [drop, keep]}), add([{url: 'guide.html', revision: '1'}]), install())
add_case('install-default-plugin-swap', 'the default cacheability plugin is added on first use and removed when a custom one is added later',
  controller({plugins: [trace('pre')]}), add(['one.html']), install(), addPlugins([keep]), add(['two.html']), network('missing'), install(), network('alpha'), add(['three.html']), install())
add_case('install-redirected', 'a redirected precache response is copied before caching (copyResponse), so the cached copy has no URL',
  controller(), add([{url: 'guide.html', revision: '1'}]), network('redirected'), install(), matchPrecache('guide.html'))
add_case('install-data-url', 'a non-http entry: Cache.put rejects after the precache was opened (B9)',
  controller(), add(['data:,x', 'after.html']), install(), keys())
add_case('install-cross-origin', 'a cross-origin entry installs through the fixture and is served by the precache route (B9)',
  app([{url: 'http://x/', revision: '1'}]), install(), activate(), network('offline'), get('http://x/'), get('http://x/?utm_campaign=y'))
add_case('install-trace', 'the plugin lifecycle of install for a cached and an uncached entry, and the event lifetimes',
  controller({plugins: [trace('pre')]}), add(['cached.html']), install(), add([{url: 'new.html', revision: '2'}]), install(), activate())
add_case('install-fail-did-respond', 'a handlerDidRespond failure rejects install although the response was already cached',
  controller({plugins: [fail('handlerDidRespond')]}), add(['one.html', 'two.html']), install())
add_case('install-shared-cache', 'two versions share one precache: v2 reuses v1 entries and its activate removes v1-only keys',
  controller(), add(site), install(), controller({}, 'v2'), add([site[0], {url: 'v2.html', revision: '1'}], 'v2'), install('v2'), activate('v2'), activate())
add_case('install-wide', 'a 160-entry manifest installs entry by entry and activates',
  controller(), add(Array.from({length: 160}, (_, i) => ({url: 'f/' + i, revision: String(i % 7)}))), install(), activate())

// matchPrecache.
add_case('match-precache', 'matchPrecache: unknown URLs open nothing, known but uninstalled URLs open the precache, a Request argument',
  controller(), add(site), matchPrecache('unknown.html'), dump, matchPrecache('guide.html'), dump, install(),
  matchPrecache('guide.html'), matchPrecache('guide.html', {request: true}), matchPrecache('guide'), matchPrecache('http://['))

// Routing and PrecacheRoute.
add_case('route-variations', 'directory index, ignored utm_ and fbclid parameters, clean URLs, exact queries',
  app(site), install(), activate(), network('offline'),
  ['docs/', 'docs/index.html?utm_source=x', 'guide', 'guide?fbclid=x', 'guide?x=y', 'a?lang=en', 'a?lang=de', 'docs/?utm_a=1&utm_b=2', 'guide.html#top'].map(url => get(url)))
add_case('route-options', 'PrecacheRoute options: no directory index, no clean URLs, custom ignore patterns with JavaScript RegExp semantics',
  controller(), add(['docs/index.html', 'guide.html', 'p.html', 'q.html?keep=1', 's.html?k=a%20b']), install(), router(),
  precacheRoute({directoryIndex: null, cleanURLs: false, ignoreURLParametersMatching: ['^ref$', '^\\d$', '^.{2}$', 'B']}), network('offline'),
  ['docs/', 'guide', 'p.html?ref=1', 'p.html?7=x', 'p.html?%D9%A3=x', 'p.html?%F0%9F%98%80=x', 'p.html?%C3%BC=x', 'p.html?ab=x', 'p.html?xBy=1&ref=2', 'p.html?xby=1',
    'q.html?keep=1&ref=1', 'p.html?utm_source=x', 's.html?k=a%20b', 's.html?k=a%20b&ref=1'].map(url => get(url)))
add_case('route-manipulation', 'urlManipulation receives the URL with ignored parameters removed and its URLs are tried last',
  controller(), add(['page.htm', 'dir/index.htm', 'page.html']), install(), router(),
  precacheRoute({directoryIndex: 'index.htm', cleanURLs: false, urlManipulation: ['.htm', '/index.htm']}), network('offline'),
  ['page', 'page?utm_x=1', 'dir/', 'dir', 'dir?fbclid=1', 'nothing'].map(url => get(url)))
add_case('route-protocol', 'the router answers only http(s) URLs: data: and about: requests are not handled (B9)',
  app(['data:,x']), defaultHandler(strategy('NetworkOnly')), get('data:,x'), get('about:blank'), get('https://example.com/'))
add_case('route-cross-origin', 'cross-origin requests reach routes with sameOrigin false and print absolute (B9)',
  app(site), install(), get('http://x/'), {op: 'route', router: 'r', crossOrigin: true, handler: strategy('NetworkOnly')}, get('http://x/'), get('https://example.com/first/a'), get('first/a'))
add_case('route-methods', 'routes are per method; the host Request normalises the six standard method names only',
  app(site), route('network/', strategy('NetworkOnly'), {method: 'POST'}), route('patch/', strategy('NetworkOnly'), {method: 'PATCH'}),
  route('head/', strategy('NetworkOnly'), {method: 'HEAD'}), install(), network('beta'),
  get('guide.html', {method: 'POST'}), get('network/x', {method: 'POST'}), get('network/x', {method: 'pOsT'}), get('guide.html', {method: 'get'}),
  get('patch/x', {method: 'patch'}), get('patch/x', {method: 'PATCH'}), get('head/x', {method: 'head'}), get('guide.html', {method: ''}), get('guide.html', {method: 'CONNECT'}))
add_case('route-precedence', 'the first registered matching route wins, for the precache route and runtime routes alike',
  controller(), add([{url: 'first/item', revision: '1'}]), install(), router(), route('first/', strategy('NetworkOnly')), precacheRoute(), network('beta'),
  get('first/item'), router('r2'), precacheRoute(undefined, {router: 'r2'}), route('first/', strategy('NetworkOnly'), {router: 'r2'}), get('first/item', {router: 'r2'}))
add_case('route-default-handler', 'default handlers per method answer requests no route matched',
  app(site), defaultHandler(strategy('NetworkOnly')), network('beta'), get('nowhere.html'), get('nowhere.html', {method: 'POST'}),
  defaultHandler(respond('posted', 201), {method: 'POST'}), get('nowhere.html', {method: 'POST'}), get('guide.html'))
add_case('route-catch-handlers', 'a route catch handler runs first, then the global one; without either the error reaches the caller',
  app(site, {runtime: [trace('rt')]}), get('only/a'), catchHandler(respond('global', 503)), get('only/b'),
  route('only2/', strategy('CacheOnly', [trace('rt2')]), {id: 'o2'}), catchHandler(strategy('CacheOnly', [trace('catch')], 'other'), {route: 'o2'}), get('only2/c'),
  catchHandler(respond('route', 504), {route: 'o2'}), get('only2/d'))
add_case('route-unregister', 'unregisterRoute removes a route; unknown routes and methods throw their WorkboxError codes',
  app(site), route('x/', strategy('NetworkOnly'), {id: 'x'}), get('x/1'), unregister('x'), get('x/2'), unregister('x'), router('empty'), unregister('x', 'empty'))
add_case('route-bound-handler', 'createHandlerBoundToURL serves one precached URL for any request; an unknown URL throws non-precached-url',
  app(site), install(), route('shell/', {bound: 'docs/index.html', controller: 'sw'}), route('bad/', {bound: 'missing.html', controller: 'sw'}),
  network('offline'), get('shell/deep/link'), get('bad/x'))
add_case('route-precache-strategy', 'the precache strategy behind a custom route has no params, so the cache key comes from getCacheKeyForURL',
  controller({plugins: [trace('pre')]}), add([{url: 'raw/a.html', revision: '1'}]), install(), router(), route('raw/', {precache: 'sw'}), network('beta'),
  get('raw/a.html'), get('raw/b.html'), get('raw/a.html?x=1'), dump)

// PrecacheStrategy outside install.
add_case('precache-network-fallback', 'a precached URL missing from the cache is fetched and, without integrity, not cached',
  app(site), install(), add([{url: 'late.html', revision: '1'}]), network('beta'), get('late.html'), get('late'), dump)
add_case('precache-integrity-repair', 'with manifest integrity the network response repairs the precache, unless the request conflicts or is no-cors',
  app([{url: 'a.html', revision: '1', integrity: ALPHA}, {url: 'b.html', revision: '1', integrity: ALPHA}, {url: 'c.html', revision: '1', integrity: ALPHA}], {plugins: [trace('pre')]}),
  get('b.html', {integrity: BETA}), get('c.html', {mode: 'no-cors'}), get('a.html'), get('a.html'), dump)
add_case('precache-no-fallback', 'fallbackToNetwork false throws missing-precache-entry for an uninstalled entry',
  controller({fallbackToNetwork: false}), add(site), router(), precacheRoute(), get('guide.html'), install(), get('guide.html'))
add_case('precache-offline-miss', 'an uncached precache entry while offline reaches the caller as plugin-error-request-will-fetch',
  app(site, {plugins: [trace('pre')]}), network('offline'), get('guide.html'))

// Runtime strategies.
add_case('cache-first', 'CacheFirst: a miss fetches and caches in the background, then hits; offline misses fail with no-response',
  app([], {runtime: [trace('rt')]}), network('offline'), get('first/item'), network('alpha'), get('first/item'), network('offline'), get('first/item'), network('beta'), get('first/item'))
add_case('cache-first-status', 'runtime strategies cache only status 200 by default; a custom cacheWillUpdate keeps a 404',
  app([]), network('missing'), get('first/a'), route('kept/', strategy('CacheFirst', [keep], 'kept')), get('kept/a'), network('alpha'), get('first/a'), get('kept/a'))
add_case('cache-only', 'CacheOnly: no-response without a cached entry; a CacheFirst with the same cache name fills it',
  app([], {runtime: [trace('rt')]}), get('only/item'), get('first/item'), router('r2'), route('first/', strategy('CacheOnly', [trace('r2')]), {router: 'r2'}),
  get('first/item', {router: 'r2'}), get('first/other', {router: 'r2'}))
add_case('network-only', 'NetworkOnly: every source, with the error nested in no-response',
  app([], {runtime: [trace('rt')]}), get('network/a'), network('beta'), get('network/a'), network('offline'), get('network/a'), network('broken'), get('network/a'), network('missing'), get('network/a'))
add_case('fallback-plugin', 'PrecacheFallbackPlugin answers a failed runtime strategy from the precache, or not at all when the fallback is absent',
  controller(), add(['offline.html']), install(), router(), precacheRoute(),
  route('net/', strategy('NetworkOnly', [trace('rt'), fallback('offline.html')])), route('none/', strategy('NetworkOnly', [fallback('absent.html')])),
  network('offline'), get('net/a'), get('none/a'), network('beta'), get('net/b'))
add_case('fail-plugins', 'a throwing plugin callback at each lifecycle point',
  app([]), network('alpha'),
  route('respond/', strategy('NetworkOnly', [trace('respond'), fail('handlerWillRespond')])), get('respond/a'),
  route('caught/', strategy('NetworkOnly', [fail('handlerWillRespond')]), {id: 'caught'}), catchHandler(respond('caught', 500), {route: 'caught'}), get('caught/a'),
  route('request/', strategy('NetworkOnly', [fail('requestWillFetch')])), get('request/a'),
  route('succeed/', strategy('NetworkOnly', [trace('succeed'), fail('fetchDidSucceed')])), get('succeed/a'),
  route('match/', strategy('CacheFirst', [fail('cachedResponseWillBeUsed')], 'match')), get('match/a'),
  route('update/', strategy('CacheFirst', [trace('update'), fail('cacheDidUpdate')], 'update')), get('update/a'), get('update/a'),
  route('key/', strategy('CacheFirst', [fail('cacheKeyWillBeUsed')], 'key')), get('key/a'),
  route('start/', strategy('NetworkOnly', [trace('start'), fail('handlerWillStart')])), get('start/a'),
  route('before/', strategy('CacheFirst', [fail('handlerWillStart'), trace('before')], 'before')), get('before/a'),
  route('will/', strategy('CacheFirst', [trace('will'), fail('cacheWillUpdate')], 'will')), get('will/a'), get('will/a'),
  network('offline'), route('error/', strategy('NetworkOnly', [fail('handlerDidError')])), get('error/a'),
  network('broken'), route('failed/', strategy('NetworkOnly', [trace('failed'), fail('fetchDidFail')])), get('failed/a'))
add_case('fail-non-error', 'a plugin or route catch handler that throws a non-Error: each instanceof Error guard ignores it, drops it or lets it through unchanged',
  app([]), network('alpha'),
  route('rwf/', strategy('NetworkOnly', [trace('rwf'), failValue('requestWillFetch')])), get('rwf/a'),
  route('fds/', strategy('NetworkOnly', [trace('fds'), failValue('fetchDidSucceed')])), get('fds/a'),
  route('fdsf/', strategy('CacheFirst', [failValue('fetchDidSucceed')], 'fdsf')), get('fdsf/a'),
  route('crw/', strategy('CacheOnly', [trace('crw'), failValue('cachedResponseWillBeUsed')])), get('crw/a'),
  route('hdr/', strategy('NetworkOnly', [trace('hdr'), failValue('handlerDidRespond')])), get('hdr/a'),
  route('cdu/', strategy('CacheFirst', [trace('cdu'), failValue('cacheDidUpdate')], 'cdu')), get('cdu/a'), get('cdu/a'),
  route('hws/', strategy('NetworkOnly', [failValue('handlerWillStart')])), get('hws/a'),
  route('rc/', strategy('CacheOnly', [trace('rc')]), {id: 'rc'}), catchHandler(strategy('CacheOnly', [failValue('cachedResponseWillBeUsed')], 'other'), {route: 'rc'}), get('rc/a'),
  route('rce/', strategy('CacheOnly'), {id: 'rce'}), catchHandler(strategy('CacheOnly', [fail('cachedResponseWillBeUsed')], 'other'), {route: 'rce'}), get('rce/a'),
  controller({plugins: [trace('pre'), failValue('handlerDidRespond')]}, 'v2'), add(['kept.html'], 'v2'), install('v2'),
  network('offline'), route('hde/', strategy('NetworkOnly', [failValue('handlerDidError')])), get('hde/a'))
add_case('network-first', 'NetworkFirst without a timeout: the network, cached in the background by its unshifted 200-or-opaque plugin, then the cache when the fetch fails',
  app([]), route('nf/', strategy('NetworkFirst', [trace('nf')], 'nf')), get('nf/a'), network('beta'), get('nf/a'), network('offline'), get('nf/a'), get('nf/b'),
  network('missing'), get('nf/c'), network('broken'), get('nf/a'), route('nfk/', strategy('NetworkFirst', [keep], 'nfk')), network('missing'), get('nfk/a'), dump)
add_case('redirected-runtime', 'runtime strategies cache a redirected response as it is; only the precache copies it',
  app([{url: 'p.html', revision: '1'}]), network('redirected'), get('first/a'), install(), dump)

// Errors in sequence, with cleanup (B9).
add_case('error-after-install', 'an invalid URL after a successful install throws, and the scenario still reports and deletes its caches (B9)',
  controller(), add([{url: 'guide.html', revision: '1'}]), install(), add([{url: 'http://[', revision: '1'}]), install(), app([]), get('http://['), get('first/a'), dump)
add_case('error-sequence', 'every step fails in turn after caches were opened; later steps still run',
  app(site), install(), network('missing'), get('first/a'), add([null]), network('offline'), get('only/a'), get('network/a'),
  add([{url: 'guide.html', revision: 'other'}]), add([{url: 'new.html', revision: '1'}]), install(), activate())
add_case('route-respond-invalid', 'invalid {respond} statuses (99, 600, -1, 65736 unsigned-short wrap to 200, 204 with non-empty vs empty/null body) reject per step with cache cleanup',
  controller(), add(['a.html']), install(), router(),
  route('low/', respond('x', 99)), route('high/', respond('x', 600)), route('neg/', respond('x', -1)),
  route('wrap/', respond('x', 65736)), route('n204bad/', respond('x', 204)), route('n204ok/', respond('', 204)), route('n204null/', respond(null, 204)),
  get('low/1'), get('high/1'), get('neg/1'), get('wrap/1'), get('n204bad/1'), get('n204ok/1'), get('n204null/1'))
add_case('route-respond-invalid-catch', 'invalid {respond} triggers route/global catch handlers, and an invalid {respond} inside a route or global catch handler propagates without hanging and cleans up caches',
  controller(), add(['a.html']), install(), router(),
  route('rc/', respond('x', 99), {id: 'rc'}), catchHandler(respond('route-caught', 200), {route: 'rc'}), get('rc/1'),
  route('gc/', respond('x', 204)), catchHandler(respond('global-caught', 200)), get('gc/1'),
  route('badrc/', strategy('CacheOnly'), {id: 'badrc'}), catchHandler(respond('x', 99), {route: 'badrc'}), get('badrc/1'),
  router('r2'), route('badgc/', strategy('CacheOnly'), {router: 'r2'}), catchHandler(respond('x', 99), {router: 'r2'}), get('badgc/1', {router: 'r2'}))
add_case('error-credentialed-url', 'a valid credentialed URL passes new URL in addToCacheList then rejects new Request with TypeError in install, matchPrecache, createHandlerBoundToURL and fetch',
  controller(), add(['a.html']), install(), add(['http://user:pass@example.com/secret']), install(),
  matchPrecache('http://user:pass@example.com/secret'), matchPrecache('http://user:pass@example.com/secret', {request: true}),
  router(), route('bound/', {bound: 'http://user:pass@example.com/secret', controller: 'sw'}), get('bound/1'), get('http://user:pass@example.com/secret'))

// The benchmark: a 12-entry precache installed and activated, then 24 fetches across the routes.
const benchManifest = Array.from({length: 12}, (_, i) => ({url: 'bench/' + (i % 3 ? 'page-' + i + '.html' : 'dir-' + i + '/index.html'), revision: 'r' + i}))
const benchPages = [1, 2, 4, 5, 7, 8, 10, 11]
const benchFetches = Array.from({length: 24}, (_, i) => {
  const round = i >> 2
  switch (i % 4) {
    case 0: return get('bench/page-' + benchPages[round] + (round % 2 ? '.html' : '?utm_source=bench'))
    case 1: return get('bench/dir-' + 3 * (round % 4) + '/')
    case 2: return get('bench/first/item-' + (i % 5))
    default: return get('bench/network/item-' + i)
  }
})
add_case('bench-cache', 'benchmark: install and activate 12 entries, then 24 fetches through precache, CacheFirst and NetworkOnly routes',
  controller(), add(benchManifest), install(), activate(), router(), precacheRoute(),
  route('bench/first/', strategy('CacheFirst')), route('bench/network/', strategy('NetworkOnly')), benchFetches)

writeFileSync(new URL('corpus.json', import.meta.url), JSON.stringify({schema: 1, library: 'workbox', cases}) + '\n')
