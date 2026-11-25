#!/usr/bin/env node
// Rust → JavaScript through mires, exactly as experiments/browser-workloads/tools/build.mjs builds
// its workloads (that file is the source of every setting below; it is imported from nowhere and
// left untouched):
//
// - compiler: scripts/build.sh --locked (a no-op when target/release is current), CARGO_INCREMENTAL
//   unset, then the keyed sysroot for the backend arguments via scripts/sysroot-cache.sh and
//   scripts/build_sysroot.sh (SKIP_BUILD=1), never a bare `cargo build` of the compiler;
// - backend: js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off (JSC_PANIC unset =
//   unwind mode), JSC_RUST_OPT_LEVEL=z, JSC_RUST_MIR_INLINE=off, js-lane=jir, js-modules=esm,
//   js-comments=off, js-entry-role=main;
// - packaging: scripts/minimal-shim.mjs --module, a one-line facade re-exporting the port's
//   `#[js_export] fn matrix_run` as `run`, Rollup with
//   the artifact-graph plugin, Terser { ecma: 2018, module, safari10, comments: false }, and the
//   scripts/es2018-gate.mjs check.
//
// The port is one crate: <port>/src/lib.rs with its modules, compiled as the program crate against
// the mires_js rlib, which is compiled here from mires-js/src with the same settings.

import {
  copyFileSync, existsSync, listFiles, main, mkdirSync, openPort, packageVersion, readFileSync, relative, repositoryRoot,
  resolve, rmSync, rollupBundle, run, sha256, stageSources, terser, versionOf, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'
import { pathToFileURL } from 'node:url'

const BACKEND_ARGS = 'js-profile=release-size js-opt=on js-ecma-target=2018 js-cleanup=off'
const LEAF_ARGS = ['js-modules=esm', 'js-comments=off', 'js-lane=jir', 'js-entry-role=main']
const TERSER = { ecma: 2018, module: true, safari10: true, format: { comments: false } }
// The compiler inputs browser-workloads hashes as its compiler tree identity.
const COMPILER_ROOTS = ['backend', 'rcgjs-object', 'jsc-link', 'mires-build', 'mires-js', 'structurizer',
  'js-macro', 'js-extern-macro', 'runtime', 'scripts', 'patches']

await main(async () => {
  const port = openPort('rust')
  const entry = port.manifest.entry || 'src/lib.rs'
  const crateName = port.manifest.crateName || `matrix_${port.library.replace(/[^a-z0-9_]/g, '_')}`
  const stage = stageSources(port)
  const compilerRoot = repositoryRoot
  const dylib = resolve(compilerRoot, `target/release/librustc_codegen_mires${process.platform === 'darwin' ? '.dylib' : '.so'}`)
  const linker = resolve(compilerRoot, 'target/release/jsc-link')

  const panic = process.env.JSC_PANIC || 'unwind'
  if (panic !== 'unwind') throw new Error('the matrix builds the default unwind mode; unset JSC_PANIC')
  const env = { ...process.env }
  delete env.CARGO_INCREMENTAL
  Object.assign(env, {
    JSC_REMAP_ROOT: stage,
    JSC_RUST_MIR_INLINE: 'off',
    JSC_RUST_OPT_LEVEL: 'z',
    JSC_SYSROOT_ARGS: BACKEND_ARGS,
    JS_EXTRA_ARGS: `${BACKEND_ARGS} js-opt-warnings=off js-workspace-root=${repositoryRoot}`,
  })
  delete env.JSC_PANIC

  const treeBefore = compilerTreeIdentity(compilerRoot)
  const binariesBefore = binaryIdentity(dylib, linker)
  run(resolve(compilerRoot, 'scripts/build.sh'), ['--locked'], { cwd: compilerRoot, env })
  const binariesAfter = binaryIdentity(dylib, linker)
  const sysroot = ensureSysroot(compilerRoot, env)
  run('cargo', ['build', '--locked', '--manifest-path', resolve(compilerRoot, 'Cargo.toml'), '-p', 'js-extern-macro'], { cwd: compilerRoot, env })

  const work = port.workDir
  const build = resolve(work, 'compile')
  rmSync(build, { recursive: true, force: true })
  const procMacroRoot = resolve(build, 'proc-macro')
  mkdirSync(procMacroRoot, { recursive: true })
  const extension = process.platform === 'darwin' ? '.dylib' : '.so'
  const procMacro = resolve(procMacroRoot, `libjs_extern_macro${extension}`)
  copyFileSync(resolve(compilerRoot, `target/debug/libjs_extern_macro${extension}`), procMacro)

  const compile = (args, leaf = []) => run(resolve(compilerRoot, 'scripts/compile_core.sh'), args, {
    cwd: compilerRoot,
    env: { ...env, JS_EXTRA_ARGS: `${env.JS_EXTRA_ARGS} ${leaf.join(' ')}`.trim() },
  })
  const miresRlib = resolve(build, 'libmires_js.rlib')
  compile([resolve(compilerRoot, 'mires-js/src/lib.rs'), miresRlib, '--crate-type', 'rlib', '--crate-name', 'mires_js',
    '--extern', `js_extern_macro=${procMacro}`])

  const program = resolve(build, 'program.js')
  const laneReport = resolve(build, 'lane-report.json')
  const optimizerReport = resolve(build, 'optimizer-report.json')
  compile([resolve(stage, entry), program, '--crate-name', crateName, '--extern', `mires_js=${miresRlib}`,
    '--search', procMacroRoot, '--search', build],
  [...LEAF_ARGS, `js-lane-report=${laneReport}`, `js-opt-report=${optimizerReport}`])
  if (!existsSync(program)) throw new Error(`mires emitted no program: ${program}`)

  const shim = resolve(build, 'shim.js')
  run(process.execPath, [resolve(compilerRoot, 'scripts/minimal-shim.mjs'), '--module', '--manifest', `${program}.artifacts.json`,
    resolve(compilerRoot, 'runtime/shim.js'), shim], { cwd: build })
  const facade = resolve(build, 'facade.js')
  // The program exports `matrix_run`; a three-letter export name could collide with a minified
  // local (the backend warns about `run`), so the facade gives it its public name.
  writeFileSync(facade, "export { matrix_run as run } from './program.js'\n")

  const { artifactGraphPlugin } = await import(pathToFileURL(resolve(compilerRoot, 'scripts/artifact-manifest.mjs')))
  let graph = null
  const bundle = await rollupBundle(facade, {
    plugins: [artifactGraphPlugin([`${program}.artifacts.json`], { onReceipt: (receipt) => { graph = receipt } })],
  })
  if (bundle.warnings.length) throw new Error(`Rollup warnings:\n${bundle.warnings.join('\n')}`)
  const readable = bundle.code
  const min = await terser(readable, TERSER)

  // ES2018 syntax gate, as browser-workloads runs it on its outputs.
  const gateDir = resolve(build, 'gate')
  mkdirSync(gateDir, { recursive: true })
  writeFileSync(resolve(gateDir, 'index.js'), readable)
  writeFileSync(resolve(gateDir, 'index.min.js'), `${min}\n`)
  run(process.execPath, [resolve(compilerRoot, 'scripts/es2018-gate.mjs'), '--parser', resolve(compilerRoot, 'node_modules/acorn/dist/acorn.mjs'),
    '--root', gateDir, '--output', resolve(gateDir, 'es2018.json'),
    '--module', resolve(gateDir, 'index.js'), '--module', resolve(gateDir, 'index.min.js')], { cwd: build, capture: true })
  const es2018 = JSON.parse(readFileSync(resolve(gateDir, 'es2018.json'), 'utf8'))

  const lane = laneSummary(JSON.parse(readFileSync(laneReport, 'utf8')))
  if (lane.ordinaryFallback !== 0) throw new Error(`mires fell back to the baseline lane for ${lane.ordinaryFallback} ordinary items`)
  const rendered = (name) => Object.entries(bundle.modules).filter(([id]) => id.endsWith(`/${name}`)).reduce((sum, [, bytes]) => sum + bytes, 0)
  const rustc = versionOf('rustc', ['--version'], { cwd: compilerRoot })
  const treeAfter = compilerTreeIdentity(compilerRoot)

  await writeDist(port, {
    min,
    readable,
    meta: {
      family: 'rust',
      variant: 'release-size',
      toolchain: {
        compiler: 'mires (rustc_codegen_mires + jsc-link)',
        backend: binariesAfter.dylib,
        linker: binariesAfter.linker,
        sysrootKey: sysroot.key,
        rustc,
        rollup: packageVersion('rollup'),
        terser: packageVersion('terser'),
      },
      settings: {
        backendArgs: BACKEND_ARGS,
        leafArgs: LEAF_ARGS,
        panic: 'unwind (JSC_PANIC unset)',
        JSC_RUST_OPT_LEVEL: 'z',
        JSC_RUST_MIR_INLINE: 'off',
        CARGO_INCREMENTAL: 'unset',
        terser: TERSER,
        packaging: 'minimal shim + facade `export { matrix_run as run } from "./program.js"`, Rollup (artifact graph), Terser',
      },
      attribution: {
        unit: 'Rollup rendered bytes before Terser',
        program: rendered('program.js'),
        runtime: rendered('shim.js'),
        facade: rendered('facade.js'),
        runtimeName: 'minimal runtime/shim.js subset this program reaches',
      },
      lane: { items: lane.total, jir: lane.jir, fallback: lane.fallback, ordinaryFallback: lane.ordinaryFallback },
      es2018: { passed: es2018.passed ?? es2018.status?.passed ?? null },
      inputGraph: graph ? sha256(JSON.stringify(graph)) : null,
    },
  })
  writeBuildInfo(port, {
    compilerRevision: run('git', ['rev-parse', 'HEAD'], { cwd: compilerRoot, capture: true }).trim(),
    compilerDirty: run('git', ['status', '--porcelain', '--', ...COMPILER_ROOTS, 'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'], { cwd: compilerRoot, capture: true }).trim() !== '',
    compilerTreeBefore: treeBefore,
    compilerTreeAfter: treeAfter,
    compilerStable: treeBefore.digest === treeAfter.digest,
    binariesBefore,
    binariesAfter,
    compilerRebuilt: binariesBefore.dylib !== binariesAfter.dylib || binariesBefore.linker !== binariesAfter.linker,
    sysroot,
    es2018,
  })
  if (treeBefore.digest !== treeAfter.digest) throw new Error('compiler inputs changed during the build')
})

function binaryIdentity(dylib, linker) {
  return {
    dylib: existsSync(dylib) ? sha256(readFileSync(dylib)) : null,
    linker: existsSync(linker) ? sha256(readFileSync(linker)) : null,
  }
}

// browser-workloads' ensureSysroot: reuse the keyed sysroot when its recorded arguments and Rust
// optimisation level match, otherwise build it with scripts/build_sysroot.sh (SKIP_BUILD=1).
function ensureSysroot(compilerRoot, env) {
  const cacheScript = resolve(compilerRoot, 'scripts/sysroot-cache.sh')
  const expectedArgs = run('bash', [cacheScript, 'args'], { cwd: compilerRoot, env, capture: true }).trim()
  const path = run('bash', [cacheScript, 'path'], { cwd: compilerRoot, env, capture: true }).trim()
  let built = false
  const current = () => {
    try {
      return readFileSync(resolve(path, '.js-args'), 'utf8').trim() === expectedArgs
        && readFileSync(resolve(path, '.rust-opt-level'), 'utf8').trim() === 'z'
    } catch {
      return false
    }
  }
  if (!current()) {
    run(resolve(compilerRoot, 'scripts/build_sysroot.sh'), [], { cwd: compilerRoot, env: { ...env, SKIP_BUILD: '1' } })
    built = true
  }
  if (!current()) throw new Error(`sysroot ${path} does not match ${expectedArgs}`)
  return { key: path.split('/').pop(), args: expectedArgs, rustOptLevel: 'z', built }
}

function compilerTreeIdentity(compilerRoot) {
  const files = []
  for (const name of COMPILER_ROOTS) files.push(...listFiles(resolve(compilerRoot, name)))
  files.push(...['Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'].map((name) => resolve(compilerRoot, name)))
  const entries = files.sort().map((path) => ({ path: relative(compilerRoot, path), sha256: sha256(readFileSync(path)) }))
  return { digest: sha256(JSON.stringify(entries)), files: entries.length }
}

function laneSummary(report) {
  const items = report.items || []
  const fallback = items.filter((item) => (item.selected_lane || item.lane) === 'baseline')
  return {
    total: items.length,
    jir: items.filter((item) => item.selected_lane === 'jir' || item.lane === 'jir').length,
    fallback: fallback.length,
    ordinaryFallback: fallback.filter((item) => item.decline !== 'encode.synthesized').length,
  }
}
