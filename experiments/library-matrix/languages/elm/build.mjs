#!/usr/bin/env node
// Elm → JavaScript, at the production settings of experiments/fingerprintjs-elm/tools/build.mjs
// (the `production` row): `elm make --optimize` (Elm 0.19.2 from the root node_modules, ELM_BIN
// overrides), the comparator's ES module wrapper, the compiler-aware minification of the
// generated Elm module only (Terser with pure_funcs F2..F9/A2..A9, pure_getters, keep_fargs off,
// unsafe and unsafe_comps, then a separate mangle pass), Rollup over the port's JS host, then the
// common Terser pass ({ comments: false, safari10, module }) on the bundle.
//
// Elm is the contract's exception: Elm 0.19 has no exports, so the port is Elm plus a JS host
// under js/ (port.json `host`, default js/index.js) that exports `run` and talks to the Elm
// program through ports only. The host imports the compiled program as `elm-program`.
//
// The wrapper is copied from experiments/fingerprintjs-elm/tools/wrap-elm.mjs with its two
// rewrites, the comparator's one emit transformation for this family:
//   1. the outgoing-port effect manager's `_Process_sleep(0)` becomes `_Scheduler_succeed(null)`,
//      so a response reaches the host's subscriber synchronously on every call (otherwise the
//      manager sleeps one timer tick after each dispatch and a second call in the same task would
//      get no response);
//   2. the IIFE's `this` binding becomes a module-local scope object, exported as the default.
// meta.json splits the Elm compiler output into program and runtime the way the comparator does:
// the runtime is an empty Platform.worker compiled the same way (`elmRuntime`).

import {
  existsSync, main, mkdirSync, openPort, packageVersion, readText, relative, removeFile, resolve, rollupBundle, rootModules,
  run, stageSources, terser, commonTerserOptions, versionOf, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'

const PURE_FUNCS = ['F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8', 'F9', 'A2', 'A3', 'A4', 'A5', 'A6', 'A7', 'A8', 'A9']
const ELM_JSON = {
  type: 'application',
  'source-directories': ['src'],
  'elm-version': '0.19.2',
  dependencies: { direct: { 'elm/core': '1.0.5', 'elm/json': '1.1.3' }, indirect: {} },
  'test-dependencies': { direct: {}, indirect: {} },
}

// Copied from experiments/fingerprintjs-elm/tools/wrap-elm.mjs (unchanged behaviour), split in two
// so the empty worker, which has no ports, can take the module rewrite alone.
function moduleWrap(source) {
  const rewritten = source.replace(/}\s*\(\s*this\s*\)\s*\)\s*;?\s*$/, '}(scope));')
  if (rewritten === source) throw new Error('could not rewrite Elm IIFE this-binding')
  return `const scope = Object.create(null);\n${rewritten}\nexport default scope.Elm;\n`
}

export function wrapElm(source) {
  const patched = source.replace('var init = _Process_sleep(0);', 'var init = _Scheduler_succeed(null);')
  if (patched === source) throw new Error('could not patch Elm outgoing-port sleep')
  return moduleWrap(patched)
}

// The comparator's compiler-aware minification of the generated Elm module.
async function minifyElm(source) {
  const compressed = await terser(source, {
    module: true,
    mangle: false,
    compress: { pure_funcs: PURE_FUNCS, pure_getters: true, keep_fargs: false, unsafe_comps: true, unsafe: true },
    format: { comments: false },
    safari10: true,
  })
  return terser(compressed, { module: true, compress: false, mangle: true, format: { comments: false }, safari10: true })
}

await main(async () => {
  const port = openPort('elm')
  const elm = process.env.ELM_BIN || resolve(rootModules, '.bin/elm')
  if (!existsSync(elm)) throw new Error(`Elm compiler missing: ${elm} (npm ci at the repository root)`)
  const mainModule = port.manifest.main || 'src/Main.elm'
  const host = port.manifest.host || 'js/index.js'
  const stage = stageSources(port)
  writeFileSync(resolve(stage, 'elm.json'), `${JSON.stringify(port.manifest.elmJson || ELM_JSON, null, 4)}\n`)
  writeFileSync(resolve(stage, 'src/MatrixEmptyWorker.elm'), `module MatrixEmptyWorker exposing (main)

import Platform


main : Program () () ()
main =
    Platform.worker
        { init = \\_ -> ( (), Cmd.none )
        , update = \\_ model -> ( model, Cmd.none )
        , subscriptions = \\_ -> Sub.none
        }
`)
  const out = resolve(port.workDir, 'out')
  mkdirSync(out, { recursive: true })
  const elmJs = resolve(out, 'elm.js')
  const emptyWorkerJs = resolve(out, 'empty-worker.js')
  removeFile(elmJs)
  removeFile(emptyWorkerJs)
  run(elm, ['make', mainModule, '--optimize', `--output=${elmJs}`], { cwd: stage })
  run(elm, ['make', 'src/MatrixEmptyWorker.elm', '--optimize', `--output=${emptyWorkerJs}`], { cwd: stage })
  const elmSource = readText(elmJs)
  const standard = wrapElm(elmSource)
  const production = `${await minifyElm(standard)}\n`
  const standardPath = resolve(out, 'elm.mjs')
  const productionPath = resolve(out, 'elm.production.mjs')
  writeFileSync(standardPath, standard)
  writeFileSync(productionPath, production)

  const alias = (target) => ({
    name: 'elm-program',
    resolveId(source) {
      return source === 'elm-program' ? target : null
    },
  })
  const entry = resolve(stage, host)
  const readable = await rollupBundle(entry, { plugins: [alias(standardPath)] })
  const bundled = await rollupBundle(entry, { plugins: [alias(productionPath)] })
  for (const bundle of [readable, bundled]) if (bundle.warnings.length) throw new Error(`Rollup warnings:\n${bundle.warnings.join('\n')}`)
  const min = await terser(bundled.code, commonTerserOptions())

  // The host's share in Rollup rendered bytes of the readable bundle, where neither the Elm module
  // nor the host is minified yet; above half, the row is labelled "mostly JS".
  let elmRendered = 0
  let hostRendered = 0
  for (const [id, bytes] of Object.entries(readable.modules)) {
    if (id === standardPath) elmRendered += bytes
    else hostRendered += bytes
  }
  const emptyWorkerSource = readText(emptyWorkerJs)
  const emptyWorkerMinified = await minifyElm(moduleWrap(emptyWorkerSource))
  const hostFiles = [host]
  await writeDist(port, {
    min,
    readable: readable.code,
    meta: {
      family: 'elm',
      variant: 'production (compiler-aware minification)',
      toolchain: { elm: versionOf(elm, ['--version']), rollup: packageVersion('rollup'), terser: packageVersion('terser') },
      settings: {
        command: 'elm make --optimize',
        elmJson: port.manifest.elmJson || ELM_JSON,
        packaging: 'comparator wrapper (outgoing-port sleep patch, module scope), compiler-aware Terser on the Elm module only, Rollup over the JS host, common Terser pass',
        elmMinifier: { pureFuncs: PURE_FUNCS, pure_getters: true, keep_fargs: false, unsafe: true, unsafe_comps: true, mangle: 'separate pass' },
        terser: commonTerserOptions(),
      },
      attribution: {
        unit: 'Rollup rendered bytes of the readable bundle (Elm module and host both unminified)',
        elmModule: elmRendered,
        jsHost: hostRendered,
        hostFiles,
        hostRenderedShare: hostRendered / (elmRendered + hostRendered),
        mostlyJsRule: 'the "mostly JS" label is computed by tools/sizes.mjs with the comparator rule (raw host-JS bytes / index.min.js > 0.5)',
        elmCompilerOutput: Buffer.byteLength(elmSource),
        elmRuntime: Buffer.byteLength(emptyWorkerSource),
        elmRuntimeMinified: Buffer.byteLength(emptyWorkerMinified),
        elmProgram: Math.max(0, Buffer.byteLength(elmSource) - Buffer.byteLength(emptyWorkerSource)),
      },
    },
  })
  writeBuildInfo(port, { elmJs, entry: relative(stage, entry) })
})
