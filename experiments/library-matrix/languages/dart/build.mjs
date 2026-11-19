#!/usr/bin/env node
// Dart → JavaScript through dart2js, at the production settings of
// experiments/fingerprintjs-dart/tools/build.mjs: `dart compile js -O2 --no-source-maps` (the
// `o2` row: safe production optimisations), then the comparator's facade and the common Terser
// pass ({ comments: false, safari10, module }). `DART` names the executable (default `dart` on
// PATH), as in the comparator runner.
//
// dart2js cannot emit ES module exports. The port's main() publishes `run` on
// globalThis.__matrixExports; the facade is the comparator's: the program runs inside a function
// that binds `self` to globalThis and `window`, `document`, `navigator`, `screen` and `location` to
// undefined (dart2js then starts main() synchronously and the import reads no browser globals),
// after which the facade reads the exports object once, deletes the global and re-exports `run`.
//
// The port holds src/*.dart with its entry (port.json `entry`, default src/main.dart). The builder
// writes pubspec.yaml (port.json `dependencies`, a pub dependency map) and runs `dart pub get`.

import {
  existsSync, globalExportsFacade, main, openPort, packageVersion, readText, removeFile, resolve, run, stageSources,
  stripSourceMapComment, terser, commonTerserOptions, versionOf, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'

const FLAGS = ['-O2', '--no-source-maps']

await main(async () => {
  const port = openPort('dart')
  const dart = process.env.DART || 'dart'
  const entry = port.manifest.entry || 'src/main.dart'
  const stage = stageSources(port)
  const dependencies = port.manifest.dependencies || {}
  const pubspec = [
    `name: matrix_${port.library.replace(/[^a-z0-9_]/g, '_')}`,
    'publish_to: none',
    'environment:',
    "  sdk: ^3.13.0",
    ...(Object.keys(dependencies).length
      ? ['dependencies:', ...Object.entries(dependencies).map(([name, version]) => `  ${name}: ${version}`)]
      : []),
    '',
  ].join('\n')
  writeFileSync(resolve(stage, 'pubspec.yaml'), pubspec)
  run(dart, ['pub', 'get', ...(Object.keys(dependencies).length ? [] : ['--offline'])], { cwd: stage })
  const output = resolve(port.workDir, 'out.js')
  removeFile(output)
  run(dart, ['compile', 'js', ...FLAGS, '-o', output, entry], { cwd: stage })
  if (!existsSync(output)) throw new Error(`dart2js produced no output: ${output}`)
  const program = stripSourceMapComment(readText(output))
  const facade = globalExportsFacade(program, {
    prefix: '(function (self, window, document, navigator, screen, location) {\n',
    suffix: '\n})(globalThis, void 0, void 0, void 0, void 0, void 0);',
  })
  const min = await terser(facade, commonTerserOptions())
  await writeDist(port, {
    min,
    readable: facade,
    meta: {
      family: 'dart',
      variant: 'o2',
      toolchain: { dart: versionOf(dart, ['--version']), terser: packageVersion('terser') },
      settings: {
        command: `dart compile js ${FLAGS.join(' ')}`,
        packaging: 'comparator facade: program in a function binding self/window/document/navigator/screen/location, exports read from globalThis.__matrixExports; common Terser pass',
        terser: commonTerserOptions(),
      },
      attribution: { unit: 'dart2js output bytes before the facade and Terser', program: Buffer.byteLength(program), runtime: null, runtimeSeparable: false },
    },
  })
  writeBuildInfo(port, { output, pubspec })
})
