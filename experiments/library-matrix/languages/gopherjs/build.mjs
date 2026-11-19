#!/usr/bin/env node
// Go → JavaScript through GopherJS, at the settings of
// experiments/fingerprintjs-gopherjs/tools/build.mjs: GopherJS 1.21.0 with Go 1.21.13 from
// .toolchains/go (experiments/tools/setup-go-comparators.sh installs both; MIRES_GO_ROOT
// overrides), `gopherjs build -m --source_map=false`, GOCACHE kept with the toolchain, then the
// comparator's facade and the common Terser pass ({ comments: false, safari10, module }).
//
// Emit policy (as for both comparator ports): the GopherJS emit ships untouched. GopherJS cannot
// emit ES modules; the port publishes `run` on js.Module.exports, and the facade evaluates the emit
// inside a function that binds `window` to globalThis and `module` to a fresh object, then
// re-exports `run`. Importing a port therefore has GopherJS's documented runtime side effects
// (globalThis.fs is defined, Error.stackTraceLimit becomes Infinity, a page-global `require` is
// called with `fs` and `util` if one exists).
//
// The port holds src/*.go (package main; port.json `package`, default ./src). The builder writes
// go.mod (module matrix/<library>, go 1.21, gopherjs v1.21.0 plus port.json `require`) and the
// shared go.sum. The runtime floor is the `floor` library's port (README "Sizes"); meta.json keeps
// the comparator's attribution of the emit into the prelude and its $packages blocks.

import {
  copyFileSync, existsSync, harnessRoot, main, mkdirSync, openPort, packageVersion, readText, removeFile,
  repositoryRoot, resolve, run, stageSources, terser, commonTerserOptions, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'

const GO_VERSION = 'go1.21.13'
const GOPHERJS_VERSION = '1.21.0'

function facade(program) {
  return `const __matrixModule = { exports: {} };
(function (window, module) {
${program}
}).call(globalThis, globalThis, __matrixModule);
export const run = __matrixModule.exports.run;
`
}

// The comparator's attribution: bytes of the `-m` emit split at its $packages[...] blocks.
function attributePackages(code) {
  const marker = /\$packages\["([^"]+)"\]\s*=\s*\(function\s*\(\)/g
  const starts = [...code.matchAll(marker)].map((match) => ({ name: match[1], index: match.index }))
  if (starts.length === 0) throw new Error('no GopherJS package blocks found')
  const end = code.indexOf('$callForAllPackages(', starts[starts.length - 1].index)
  const packages = {}
  starts.forEach((start, i) => {
    packages[start.name] = (i + 1 < starts.length ? starts[i + 1].index : end) - start.index
  })
  return { prelude: starts[0].index, packages, startup: code.length - end }
}

await main(async () => {
  const port = openPort('gopherjs')
  const goBase = process.env.MIRES_GO_ROOT || resolve(repositoryRoot, '.toolchains/go')
  const goRoot = resolve(goBase, GO_VERSION)
  const gopherjs = resolve(goBase, 'bin/gopherjs')
  if (!existsSync(gopherjs)) throw new Error(`GopherJS not found at ${gopherjs}; run experiments/tools/setup-go-comparators.sh`)
  const env = {
    ...process.env,
    GOROOT: goRoot,
    GOPHERJS_GOROOT: goRoot,
    GOPATH: resolve(goBase, 'gopath'),
    GOCACHE: resolve(goBase, 'cache'),
    GOTOOLCHAIN: 'local',
    GOFLAGS: '-modcacherw',
    PATH: `${resolve(goRoot, 'bin')}:${process.env.PATH || ''}`,
  }
  const version = run(gopherjs, ['version'], { env, capture: true }).trim()
  if (version !== `GopherJS ${GOPHERJS_VERSION}+${GO_VERSION}`) throw new Error(`expected GopherJS ${GOPHERJS_VERSION}+${GO_VERSION}, got ${version}`)

  const stage = stageSources(port)
  const requires = { 'github.com/gopherjs/gopherjs': 'v1.21.0', ...(port.manifest.require || {}) }
  writeFileSync(resolve(stage, 'go.mod'), `module matrix/${port.library}\n\ngo 1.21\n\n${Object.entries(requires).map(([path, v]) => `require ${path} ${v}`).join('\n')}\n`)
  copyFileSync(resolve(harnessRoot, 'languages/gopherjs/go.sum'), resolve(stage, 'go.sum'))
  if (port.manifest.goSum) writeFileSync(resolve(stage, 'go.sum'), `${readText(resolve(stage, 'go.sum'))}${readText(resolve(port.portDir, port.manifest.goSum))}`)

  const out = resolve(port.workDir, 'out')
  mkdirSync(out, { recursive: true })
  const build = (pkg, output) => {
    removeFile(output)
    run(gopherjs, ['build', '-m', '--source_map=false', '-o', output, pkg], { cwd: stage, env })
    if (!existsSync(output)) throw new Error(`GopherJS produced no output: ${output}`)
    return readText(output)
  }
  const program = build(port.manifest.package || './src', resolve(out, 'program.js'))
  const readable = facade(program)
  const min = await terser(readable, commonTerserOptions())
  await writeDist(port, {
    min,
    readable,
    meta: {
      family: 'gopherjs',
      variant: 'default (-m)',
      toolchain: { gopherjs: version, terser: packageVersion('terser') },
      settings: {
        command: 'gopherjs build -m --source_map=false',
        packaging: 'comparator facade: emit untouched inside a function binding window=globalThis and module; re-export of module.exports.run; common Terser pass',
        terser: commonTerserOptions(),
      },
      attribution: { unit: 'gopherjs -m emit bytes before the facade and Terser', emit: program.length, ...attributePackages(program) },
    },
  })
  writeBuildInfo(port, { goBase })
})
