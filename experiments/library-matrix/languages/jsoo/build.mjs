#!/usr/bin/env node
// OCaml → JavaScript through js_of_ocaml, at the settings of
// experiments/fingerprintjs-jsoo/tools/build.mjs: js_of_ocaml 6.4.1 in the local opam switch
// (languages/opam.mjs), a Dune `executable` in `js` mode, whole-program compilation, no source map,
// flags --effects=disabled --target-env=browser --no-extern-fs --opt=3, then the common Terser pass
// ({ comments: false, safari10, module }).
//
// js_of_ocaml emits a classic script. The port publishes `run` with `Js.export`, which writes to
// the program's `jsoo_exports` object: `module.exports` when a CommonJS `module` is in scope,
// otherwise the global object. The generated facade evaluates the emit inside a function that binds
// `module` to a fresh object (loading-only packaging, as for GopherJS), so nothing is written to
// globalThis, and re-exports `run`. meta.json attributes the emit as the comparator does: the
// runtime is the emit minus a `--no-runtime` compile of the same bytecode.

import {
  existsSync, main, mkdirSync, openPort, packageVersion, readText, resolve, stageSources, stripSourceMapComment, terser,
  commonTerserOptions, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'
import { opamExec, opamRoot, opamSwitch, opamVersion, requireSwitch } from '../opam.mjs'

const FLAGS = ['--effects=disabled', '--target-env=browser', '--no-extern-fs', '--opt=3']

await main(async () => {
  const port = openPort('jsoo')
  requireSwitch()
  const entry = port.manifest.entry || 'src/main.ml'
  const executable = entry.replace(/^src\//, '').replace(/\.ml$/, '')
  const stage = stageSources(port)
  writeFileSync(resolve(stage, 'dune-project'), '(lang dune 3.17)\n')
  mkdirSync(resolve(stage, 'src'), { recursive: true })
  const libraries = ['js_of_ocaml', ...(port.manifest.libraries || [])].join(' ')
  writeFileSync(resolve(stage, 'src/dune'), `(executable
 (name ${executable})
 (modes js)
 (libraries ${libraries})
 (preprocess
  (pps js_of_ocaml-ppx))
 (js_of_ocaml
  (compilation_mode whole_program)
  (sourcemap no)
  (flags
   ${FLAGS.join('\n   ')})))
`)
  opamExec(['dune', 'build', '--root', stage, `./src/${executable}.bc.js`], { cwd: stage })
  const emitted = resolve(stage, `_build/default/src/${executable}.bc.js`)
  const bytecode = resolve(stage, `_build/default/src/${executable}.bc-for-jsoo`)
  if (!existsSync(emitted)) throw new Error(`js_of_ocaml emit missing: ${emitted}`)
  const program = stripSourceMapComment(readText(emitted))

  // Runtime attribution, as experiments/fingerprintjs-jsoo/tools/build.mjs measures it.
  // The attribution is diagnostic only; it never decides whether the port built. js_of_ocaml 6.4.1
  // refuses `--no-runtime` for any program that keeps an integer multiply, divide or modulo
  // ("Unresolved internal primitive: %int_mul", likewise %int_div / %int_mod; repro in
  // build/library-matrix/jsoo-bugs/no-runtime-int-arith/), so a failed attribution compile is
  // recorded as `runtime: null` with its reason. The uniform runtime floor is the `floor` library.
  let runtimeBytes = null
  let runtimeUnavailable = null
  if (existsSync(bytecode)) {
    const programOnly = resolve(stage, '_build/program-only.js')
    try {
      opamExec(['js_of_ocaml', 'compile', '--no-runtime', '--no-source-map', ...FLAGS, '-o', programOnly, bytecode], { cwd: stage, capture: true })
      runtimeBytes = Buffer.byteLength(program) - Buffer.byteLength(stripSourceMapComment(readText(programOnly)))
    } catch (error) {
      // run() prints the compiler's stderr into the build log and throws only the exit status. Record
      // no paths here, so meta.json stays independent of the checkout location. The catch is broad;
      // an attribution failure is visible as `runtime: null` plus this field in sizes and meta.json.
      const exit = /\(exit [^)]*\)/.exec(String(error?.message ?? error))?.[0] ?? '(no exit status)'
      runtimeUnavailable = `js_of_ocaml compile --no-runtime failed ${exit}; the compiler's stderr is in the build log`
    }
  }

  const facade = `const __matrixModule = { exports: {} };
(function (module) {
${program}
})(__matrixModule);
export const run = __matrixModule.exports.run;
`
  const min = await terser(facade, commonTerserOptions())
  await writeDist(port, {
    min,
    readable: facade,
    meta: {
      family: 'jsoo',
      variant: 'whole-program --opt=3',
      toolchain: {
        jsOfOcaml: opamVersion(['js_of_ocaml', '--version']),
        ocaml: opamVersion(['ocamlfind', 'ocamlopt', '-version']),
        dune: opamVersion(['dune', '--version']),
        opamSwitch,
        terser: packageVersion('terser'),
      },
      settings: {
        compilationMode: 'whole_program',
        flags: FLAGS,
        packaging: 'facade binds `module` for Js.export and re-exports run; common Terser pass',
        terser: commonTerserOptions(),
      },
      attribution: {
        unit: 'emit bytes before the facade and Terser',
        emit: Buffer.byteLength(program),
        runtime: runtimeBytes,
        program: runtimeBytes === null ? null : Buffer.byteLength(program) - runtimeBytes,
        runtimeName: 'js_of_ocaml runtime linked into the whole-program emit',
        ...(runtimeUnavailable ? { runtimeUnavailable } : {}),
      },
    },
  })
  writeBuildInfo(port, { emitted, opamRoot })
})
