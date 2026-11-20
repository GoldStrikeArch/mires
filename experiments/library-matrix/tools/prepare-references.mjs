// These two npm releases publish only CommonJS/UMD. Put an ESM loading facade
// beside each installed source, so Node and WebDriver execute identical bytes.
// Both the source and the facade are included in the harness's package digest.
import { readFileSync, writeFileSync } from 'node:fs'

for (const [name, entry] of [['lz-string','libs/lz-string.js'], ['jmespath','jmespath.js']]) {
  const dir = new URL('../node_modules/' + name + '/', import.meta.url)
  const source = readFileSync(new URL(entry,dir),'utf8')
  const facade = 'const module = { exports: {} };\n(function(module, exports, define) {\n' +
    source + '\n})(module, module.exports, undefined);\nexport default module.exports;\n'
  writeFileSync(new URL('matrix-entry.mjs',dir),facade)
}

// Workbox publishes production bundles for importScripts. Load the exact bundles
// in dependency order in a private module namespace. FetchEvent is a native
// global in a service worker. The page driver uses Workbox's {event,request}
// overload, which still checks instanceof FetchEvent before reading its options.
{
  const names = ['core', 'routing', 'strategies', 'precaching']
  const root = new URL('../node_modules/', import.meta.url)
  let code = 'const workbox = {};\nconst scope = {workbox};\nconst FetchEvent = globalThis.FetchEvent ?? class FetchEvent {};\n'
  for (const name of names) {
    const pkg = 'workbox-' + name
    const source = readFileSync(new URL(pkg + '/build/' + pkg + '.prod.js', root), 'utf8')
    code += '(function(){\n' + source + '\n}).call(scope);\n'
  }
  writeFileSync(new URL('workbox-precaching/matrix-entry.mjs', root), code + 'export default workbox;\n')
}
