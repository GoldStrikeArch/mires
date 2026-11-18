#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import vm from 'node:vm'

const here = dirname(fileURLToPath(import.meta.url))
const compareRoot = resolve(here, '..')
const repoRoot = resolve(compareRoot, '../..')
const registry = JSON.parse(await readFile(resolve(compareRoot, 'ports.json'), 'utf8'))
const frozen = JSON.parse(
  await readFile(resolve(repoRoot, 'experiments/fingerprintjs-rust/oracle/frozen.json'), 'utf8'),
)

const options = parseArgs(process.argv.slice(2))
const rows = [registry.oracle, ...registry.ports]
const oracle = await inspectPort(registry.oracle)
const expectedTypesHash = frozen.files['fp.d.ts'].sha256
const results = []

for (const port of rows) {
  try {
    const inspected = port.id === registry.oracle.id ? oracle : await inspectPort(port)
    const namespaceExact = same(inspected.exportKeys, frozen.exports)
    const defaultKeysExact = same(inspected.defaultKeys, frozen.defaultExports)
    const defaultIdentityExact = frozen.defaultExports.every(
      (key) => inspected.defaultIdentity[key] === true,
    )
    const sourceRegistryExact = same(inspected.sourceKeys, frozen.sources)
    const sourceMetadataDifferences = frozen.sources.flatMap((key) => {
      const expected = oracle.sourceMetadata[key]
      const actual = inspected.sourceMetadata[key]
      return same(expected, actual) ? [] : [{ key, expected, actual }]
    })
    const exportMetadataDifferences = frozen.exports.flatMap((key) => {
      if (key === 'default') return []
      const expected = oracle.exportMetadata[key]
      const actual = inspected.exportMetadata[key]
      return same(expected, actual) ? [] : [{ key, expected, actual }]
    })
    const declarationsExact = inspected.declarationsHash === expectedTypesHash
    const contractApiExact =
      namespaceExact &&
      defaultKeysExact &&
      defaultIdentityExact &&
      sourceRegistryExact &&
      sourceMetadataDifferences.length === 0 &&
      declarationsExact
    const strictApiExact = contractApiExact && exportMetadataDifferences.length === 0
    const iifeNamespaceExact = same(inspected.iife, oracle.iife)
    const packageFormatsExact = same(inspected.artifacts, oracle.artifacts)
    const packageSurfaceComplete = Object.entries(oracle.artifacts).every(
      ([file, required]) => !required || inspected.artifacts[file] === true,
    )

    results.push({
      id: port.id,
      label: port.label,
      family: port.family,
      profile: port.profile,
      runtimeDefault: port.runtimeDefault === true,
      dist: resolve(compareRoot, port.dist),
      namespaceExact,
      defaultKeysExact,
      defaultIdentityExact,
      sourceRegistryExact,
      sourceMetadataDifferences,
      exportMetadataDifferences,
      declarationsExact,
      declarationsHash: inspected.declarationsHash,
      contractApiExact,
      strictApiExact,
      packageFormatsExact,
      packageSurfaceComplete,
      iifeNamespaceExact,
      iife: inspected.iife,
      exportKeys: inspected.exportKeys,
      defaultKeys: inspected.defaultKeys,
      sourceKeys: inspected.sourceKeys,
      sourceMetadata: inspected.sourceMetadata,
      exportMetadata: inspected.exportMetadata,
      artifacts: inspected.artifacts,
      imports: inspected.imports,
    })
  } catch (error) {
    results.push({
      id: port.id,
      label: port.label,
      family: port.family,
      profile: port.profile,
      runtimeDefault: port.runtimeDefault === true,
      dist: resolve(compareRoot, port.dist),
      importError: String(error),
      contractApiExact: false,
      strictApiExact: false,
      packageFormatsExact: false,
      packageSurfaceComplete: false,
      iifeNamespaceExact: false,
    })
  }
}

const report = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  oracleCommit: frozen.oracle.commit,
  expected: {
    exports: frozen.exports,
    defaultExports: frozen.defaultExports,
    sources: frozen.sources,
    declarationsSha256: expectedTypesHash,
    oracleArtifacts: oracle.artifacts,
    oracleExportMetadata: oracle.exportMetadata,
    oracleSourceMetadata: oracle.sourceMetadata,
    oracleIife: oracle.iife,
  },
  results,
}

if (options.output) {
  await writeFile(resolve(process.cwd(), options.output), `${JSON.stringify(report, null, 2)}\n`)
}

printTable(results)

if (options.strict && results.some((row) => !row.strictApiExact || !row.packageSurfaceComplete)) {
  process.exitCode = 2
}

async function inspectPort(port) {
  const dist = resolve(compareRoot, port.dist)
  const esmPath = resolve(dist, 'fp.esm.js')
  const dtsPath = resolve(dist, 'fp.d.ts')
  const module = await import(`${pathToFileURL(esmPath)}?static-parity=${Date.now()}-${Math.random()}`)
  const esmText = await readFile(esmPath, 'utf8')
  const sourceMetadata = Object.fromEntries(
    Object.entries(module.sources ?? {}).map(([key, value]) => [
      key,
      {
        type: typeof value,
        name: typeof value === 'function' ? value.name : undefined,
        length: typeof value === 'function' ? value.length : undefined,
      },
    ]),
  )
  const exportMetadata = Object.fromEntries(
    Object.keys(module).map((key) => [
      key,
      {
        type: typeof module[key],
        name: typeof module[key] === 'function' ? module[key].name : undefined,
        length: typeof module[key] === 'function' ? module[key].length : undefined,
      },
    ]),
  )
  const defaultIdentity = Object.fromEntries(
    frozen.defaultExports.map((key) => [key, module.default?.[key] === module[key]]),
  )
  const artifacts = Object.fromEntries(
    [
      'fp.esm.js',
      'fp.esm.min.js',
      'fp.cjs.js',
      'fp.js',
      'fp.min.js',
      'fp.umd.js',
      'fp.umd.min.js',
      'fp.d.ts',
    ].map((file) => [file, existsSync(resolve(dist, file))]),
  )

  return {
    exportKeys: Object.keys(module),
    defaultKeys: Object.keys(module.default ?? {}),
    sourceKeys: Object.keys(module.sources ?? {}),
    sourceMetadata,
    exportMetadata,
    defaultIdentity,
    declarationsHash: existsSync(dtsPath) ? sha256(await readFile(dtsPath)) : null,
    artifacts,
    iife: await inspectIife(dist),
    imports: [
      ...esmText.matchAll(/(?:from\s*|import\s*)["']([^"']+)["']/g),
    ].map((match) => match[1]),
  }
}


async function inspectIife(dist) {
  const path = resolve(dist, 'fp.min.js')
  if (!existsSync(path)) return { present: false }
  // Only host APIs a bare V8 context lacks are injected. The context keeps its own
  // intrinsics (Object, Array, Promise, ...): injecting the host's would give the
  // program a split-realm global no browser has, where `[]` and `Array` come from
  // different realms, which breaks emits that compare constructors (GopherJS).
  const sandbox = {
    console,
    setTimeout,
    clearTimeout,
    TextEncoder,
    TextDecoder,
    MessageChannel,
    performance,
    URL,
    URLSearchParams,
  }
  sandbox.globalThis = sandbox
  sandbox.self = sandbox
  sandbox.window = sandbox
  vm.runInNewContext(await readFile(path, 'utf8'), sandbox, { filename: path })
  const namespace = sandbox.FingerprintJS
  if (!namespace) return { present: true, missingNamespace: true }
  return {
    present: true,
    exportKeys: Object.keys(namespace),
    defaultKeys: Object.keys(namespace.default ?? {}),
    sourceKeys: Object.keys(namespace.sources ?? {}),
    defaultIdentity: Object.fromEntries(
      frozen.defaultExports.map((key) => [key, namespace.default?.[key] === namespace[key]]),
    ),
  }
}

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex')
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right)
}

function printTable(rows) {
  const header = [
    pad('implementation', 24),
    pad('contract API', 12),
    pad('strict API', 10),
    pad('formats', 9),
    pad('metadata differences', 20),
  ].join(' ')
  console.log(header)
  for (const row of rows) {
    console.log(
      [
        pad(row.id, 24),
        pad(row.contractApiExact ? 'PASS' : 'FAIL', 12),
        pad(row.strictApiExact ? 'PASS' : 'FAIL', 10),
        pad(row.packageSurfaceComplete ? 'PASS' : 'MISSING', 9),
        pad(String(row.exportMetadataDifferences?.length ?? 'import error'), 20),
      ].join(' '),
    )
  }
}

function pad(value, width) {
  const text = String(value)
  return text.length >= width ? text.slice(0, width) : text.padEnd(width)
}

function parseArgs(args) {
  const result = { output: null, strict: false }
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]
    if (value === '--output') result.output = args[++index]
    else if (value === '--strict') result.strict = true
    else throw new Error(`unknown argument: ${value}`)
  }
  return result
}
