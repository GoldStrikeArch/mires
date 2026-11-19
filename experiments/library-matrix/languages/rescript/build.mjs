#!/usr/bin/env node
// ReScript → JavaScript, at the settings of experiments/fingerprintjs-rescript/tools/build.mjs:
// ReScript 12.3.0 from the root node_modules, ES module output (`package-specs.module: esmodule`,
// suffix `.res.mjs`), Rollup with @rollup/plugin-node-resolve (browser, import, default) to bundle
// the @rescript/runtime modules the emit imports, then the common Terser pass
// ({ comments: false, safari10, module }).
//
// The port holds `src/*.res`; its entry module (port.json `entry`, default src/Index.res) must
// define `run`. The builder generates rescript.json and package.json in the work directory.

import {
  existsSync, main, openPort, packageVersion, resolve, rollupBundle, nodeResolvePlugin, rootModules, run, stageSources,
  terser, commonTerserOptions, versionOf, writeBuildInfo, writeDist, writeFileSync, relative,
} from '../common.mjs'

await main(async () => {
  const port = openPort('rescript')
  const entry = port.manifest.entry || 'src/Index.res'
  const stage = stageSources(port)
  const name = `matrix-${port.library}-rescript`
  writeFileSync(resolve(stage, 'package.json'), `${JSON.stringify({ name, private: true, version: '0.0.0' }, null, 2)}\n`)
  writeFileSync(resolve(stage, 'rescript.json'), `${JSON.stringify({
    name,
    sources: [{ dir: 'src', subdirs: true }],
    'package-specs': { module: 'esmodule', 'in-source': false },
    suffix: '.res.mjs',
    ...(port.manifest.rescript || {}),
  }, null, 2)}\n`)

  const compiler = resolve(rootModules, '.bin/rescript')
  run(compiler, [], { cwd: stage })
  const emitted = resolve(stage, 'lib/es6', entry.replace(/\.res$/, '.res.mjs'))
  if (!existsSync(emitted)) throw new Error(`ReScript emit missing: ${emitted}`)

  const bundle = await rollupBundle(emitted, { plugins: [await nodeResolvePlugin()] })
  if (bundle.warnings.length) throw new Error(`Rollup warnings:\n${bundle.warnings.join('\n')}`)
  const min = await terser(bundle.code, commonTerserOptions())

  let program = 0
  let runtime = 0
  for (const [id, bytes] of Object.entries(bundle.modules)) {
    if (id.includes('/node_modules/@rescript/runtime/')) runtime += bytes
    else program += bytes
  }
  await writeDist(port, {
    min,
    readable: bundle.code,
    meta: {
      family: 'rescript',
      variant: 'default',
      toolchain: {
        compiler: versionOf(compiler, ['--version']),
        rollup: packageVersion('rollup'),
        nodeResolve: packageVersion('@rollup/plugin-node-resolve'),
        terser: packageVersion('terser'),
      },
      settings: {
        packageSpecs: { module: 'esmodule', 'in-source': false },
        packaging: 'Rollup + node-resolve (browser) bundles the emit and its @rescript/runtime imports; common Terser pass',
        terser: commonTerserOptions(),
      },
      attribution: { unit: 'Rollup rendered bytes before Terser', program, runtime, runtimeName: '@rescript/runtime modules' },
      modules: Object.fromEntries(Object.entries(bundle.modules).map(([id, bytes]) => [relative(stage, id).replace(/^(\.\.\/)+/, ''), bytes])),
    },
  })
  writeBuildInfo(port, { emitted })
})
