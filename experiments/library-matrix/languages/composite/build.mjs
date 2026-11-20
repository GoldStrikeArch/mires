// Compile the native application driver, then link its exported run with the
// production exports of the same-language components into one standalone ESM.
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, relative, resolve } from 'node:path'
import {
  commonTerserOptions,
  harnessRoot,
  main,
  openPort,
  parseArgs,
  rollupBundle,
  run,
  sha256,
  terser,
  writeBuildInfo,
  writeDist
} from '../common.mjs'

import { componentsOf, componentSources, lockPath, verifyComponentLock } from '../../tools/lib/composite.mjs'
import {
  artifactOf,
  freshness,
  loadRegistry,
  readJson,
  receiptPaths,
  receiptsRoot,
  stableJson,
  writeJson
} from '../../tools/lib/registry.mjs'

await main(async () => {
  const args = parseArgs()
  const registry = loadRegistry(args.matrix)
  const paths = receiptPaths(receiptsRoot(args))
  const port = registry.ports.find((p) => p.dir === resolve(args.port))
  if (!port) throw new Error('The composite application must be present in the matrix registry')
  const language = port.language
  const app = openPort(language)
  const components = componentsOf(registry, port)
  const builder = registry.languages[language].builderPath
  const sources = componentSources(registry, port, components)
  const stage = resolve(app.workDir, 'source')
  compile(app.portDir, stage, resolve(app.workDir, 'compile'))
  const driverMeta = readJson(resolve(stage, 'dist/meta.json'))
  const inputs = {}
  const artifacts = {}
  const origins = {}
  for (const dep of components) {
    const state = freshness(registry, paths, dep)
    const reuse =
      state.fresh &&
      stableJson(readJson(resolve(dep.dir, 'dist/meta.json')).toolchain) === stableJson(driverMeta.toolchain)
    const snapshot = resolve(app.workDir, 'components', dep.library)
    const source = reuse ? dep.dir : resolve(app.workDir, 'private', dep.library, 'source')
    if (!reuse) {
      compile(dep.dir, source, resolve(app.workDir, 'private', dep.library, 'compile'))
      const emitted = readJson(resolve(source, 'dist/meta.json'))
      if (stableJson(emitted.toolchain) !== stableJson(driverMeta.toolchain)) {
        throw new Error(`${dep.key}: compiler toolchain changed during the composite build`)
      }
    }
    mkdirSync(snapshot, { recursive: true })
    const digests = {}
    // Link a snapshot of the exact files verified here, even during a concurrent standalone rebuild.
    for (const [name, field] of [
      ['index.js', 'readable'],
      ['index.min.js', 'production']
    ]) {
      const bytes = readFileSync(resolve(source, 'dist', name))
      digests[field] = sha256(bytes)
      if (reuse && digests[field] !== artifactOf(state.receipt, name)?.sha256) {
        throw new Error(`${dep.key}: ${name} changed during the composite build`)
      }
      writeFileSync(resolve(snapshot, name), bytes)
    }
    inputs[dep.library] = snapshot
    artifacts[dep.library] = digests
    origins[dep.library] = reuse ? 'verified standalone build' : 'private source build'
  }
  const current = { ...sources, toolchain: driverMeta.toolchain, artifacts }
  if (stableJson(sources) !== stableJson(componentSources(registry, port, components))) {
    throw new Error(`${port.key}: component inputs changed during the build`)
  }
  if (args['lock-only']) {
    writeJson(lockPath(port), current)
    console.log(`Locked ${port.key} to ${components.length} component builds`)
    return
  }
  verifyComponentLock(port, current)
  const bundles = {}
  const records = {}
  for (const name of ['index.js', 'index.min.js']) {
    const paths = {
      driver: resolve(stage, 'dist', name),
      ...Object.fromEntries(Object.entries(inputs).map(([id, dir]) => [id, resolve(dir, name)]))
    }
    const entry = resolve(app.workDir, 'link-' + name)
    const lines = Object.values(paths).map(
      (file, index) =>
        `import {run as module${index}} from ${JSON.stringify('./' + relative(dirname(entry), file))}`
    )
    const properties = Object.keys(inputs).map((key, i) => `${JSON.stringify(key)}:module${i + 1}`)
    lines.push(
      `const components = {${properties.join(',')}}`,
      `
export async function run(input) {
  const history = [];
  for (;;) {
    const [name, argument] = module0([input, history]);
    if (name === null) return argument;
    let value = components[name](argument);
    if (value && typeof value.then === "function") value = await value;
    history.push(ArrayBuffer.isView(value) ? Array.from(value) : value);
  }
}`
    )
    writeFileSync(entry, lines.join('\n') + '\n')
    // GopherJS uses labelled breaks for Go control flow. Rollup's tree shaker can
    // delete a reachable label when relinking the already optimized readable modules.
    const bundle = await rollupBundle(entry, { treeshake: language !== 'gopherjs' })
    if (bundle.warnings.length) throw new Error(bundle.warnings.join('\n'))
    bundles[name] = bundle.code
    records[name] = Object.fromEntries(
      Object.entries(paths).map(([id, path]) => [id, sha256(readFileSync(path))])
    )
  }
  verifyComponentLock(port, {
    ...componentSources(registry, port, components),
    toolchain: driverMeta.toolchain,
    artifacts
  })
  await writeDist(app, {
    readable: bundles['index.js'],
    min: await terser(bundles['index.min.js'], commonTerserOptions()),
    meta: {
      family: driverMeta.family,
      variant: driverMeta.variant,
      toolchain: driverMeta.toolchain,
      settings: {
        packaging: 'Native application driver and same-language production modules, Rollup and Terser',
        treeShaking: language !== 'gopherjs'
      },
      components: records,
      componentSources: sources.sources,
      componentArtifacts: artifacts
    }
  })
  writeBuildInfo(app, {
    driver: JSON.parse(readFileSync(resolve(app.workDir, 'compile/build-info.json'), 'utf8')),
    components: records,
    origins
  })

  function compile(from, to, work) {
    rmSync(to, { recursive: true, force: true })
    cpSync(from, to, { recursive: true, filter: (path) => path !== resolve(from, 'dist') })
    run(process.execPath, [builder, '--port', to, '--work', work])
  }
})
