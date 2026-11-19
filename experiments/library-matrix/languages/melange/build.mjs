#!/usr/bin/env node
// OCaml → JavaScript through Melange, at the settings of
// experiments/fingerprintjs-melange/tools/build.mjs: Melange 7.0.1 in the local opam switch
// (languages/opam.mjs), `melange.emit` with ES6 modules and melange.ppx, Rollup, then the common
// Terser pass ({ comments: false, safari10, module }).
//
// One difference from the comparator build, needed by ordinary OCaml code: the comparator sets
// `(emit_stdlib false)` because its emit is import-free; a port that uses the OCaml or Melange
// stdlib (List, Hashtbl, Js.Map, ...) imports those modules, so the matrix emits them
// (`emit_stdlib true`, Melange's default) and bundles them with @rollup/plugin-node-resolve. They
// are counted as the runtime in meta.json.
//
// The port holds src/*.ml; the entry module (port.json `entry`, default src/index.ml) defines `run`.

import {
  existsSync, main, mkdirSync, nodeResolvePlugin, openPort, packageVersion, relative, resolve, rollupBundle, stageSources,
  terser, commonTerserOptions, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'
import { opamExec, opamRoot, opamSwitch, opamVersion, requireSwitch } from '../opam.mjs'

await main(async () => {
  const port = openPort('melange')
  requireSwitch()
  const entry = port.manifest.entry || 'src/index.ml'
  const stage = stageSources(port)
  writeFileSync(resolve(stage, 'dune-project'), '(lang dune 3.16)\n\n(using melange 0.1)\n')
  mkdirSync(resolve(stage, 'src'), { recursive: true })
  const libraries = (port.manifest.libraries || []).join(' ')
  writeFileSync(resolve(stage, 'src/dune'), `(melange.emit
 (target out)
 (alias default)
 (module_systems es6)
 (emit_stdlib true)${libraries ? `\n (libraries ${libraries})` : ''}
 (preprocess
  (pps melange.ppx)))
`)
  opamExec(['dune', 'build', '@default', '--root', stage], { cwd: stage })
  const emitted = resolve(stage, '_build/default/src/out', entry.replace(/\.ml$/, '.js'))
  if (!existsSync(emitted)) throw new Error(`Melange emit missing: ${emitted}`)

  const bundle = await rollupBundle(emitted, { plugins: [await nodeResolvePlugin()] })
  if (bundle.warnings.length) throw new Error(`Rollup warnings:\n${bundle.warnings.join('\n')}`)
  const min = await terser(bundle.code, commonTerserOptions())

  let program = 0
  let runtime = 0
  for (const [id, bytes] of Object.entries(bundle.modules)) {
    if (id.includes('/node_modules/')) runtime += bytes
    else program += bytes
  }
  await writeDist(port, {
    min,
    readable: bundle.code,
    meta: {
      family: 'melange',
      variant: 'default',
      toolchain: {
        melange: opamVersion(['melc', '--version']),
        ocaml: opamVersion(['ocamlfind', 'ocamlopt', '-version']),
        dune: opamVersion(['dune', '--version']),
        opamSwitch,
        rollup: packageVersion('rollup'),
        nodeResolve: packageVersion('@rollup/plugin-node-resolve'),
        terser: packageVersion('terser'),
      },
      settings: {
        dune: 'melange.emit (module_systems es6) (emit_stdlib true) (preprocess (pps melange.ppx))',
        packaging: 'Rollup + node-resolve bundles the emit and the emitted stdlib modules; common Terser pass',
        terser: commonTerserOptions(),
      },
      attribution: { unit: 'Rollup rendered bytes before Terser', program, runtime, runtimeName: 'emitted OCaml/Melange stdlib modules (melange, melange.js)' },
      modules: Object.fromEntries(Object.entries(bundle.modules).map(([id, bytes]) => [relative(resolve(stage, '_build/default/src/out'), id), bytes])),
    },
  })
  writeBuildInfo(port, { emitted, opamRoot })
})
