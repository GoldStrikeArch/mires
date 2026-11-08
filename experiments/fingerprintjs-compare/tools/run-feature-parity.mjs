#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { startServer } from '../../fingerprintjs-rust/tools/static-server.mjs'
import { compareContracts } from '../../fingerprintjs-rust/tests/compare.mjs'
import { browserIdentity, createDriver } from '../../fingerprintjs-rust/tests/webdriver.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const compareRoot = resolve(here, '..')
const repoRoot = resolve(compareRoot, '../..')
const oracleRoot = resolve(repoRoot, 'experiments/fingerprintjs')
const registry = JSON.parse(await readFile(resolve(compareRoot, 'ports.json'), 'utf8'))
const options = parseArgs(process.argv.slice(2))
const portsById = new Map([registry.oracle, ...registry.ports].map((port) => [port.id, port]))
const canonicalIds = [
  'rust-idiomatic',
  'rescript',
  'melange',
  'jsoo',
  'rust-compact',
  'rust-hybrid',
  'dart-o2',
  'scalajs-full',
  'elm-production',
  'kotlin-production',
]
const selectedIds = options.allVariants
  ? registry.ports.map((port) => port.id)
  : options.ports.length
    ? options.ports
    : canonicalIds
const selectedPorts = selectedIds.map((id) => {
  const port = portsById.get(id)
  if (!port) throw new Error(`unknown port: ${id}`)
  if (id === registry.oracle.id) throw new Error('the oracle is the comparison baseline, not a candidate')
  return port
})
const contract = await readFile(resolve(here, 'feature-parity-contract.js'))
const upstream = resolve(compareRoot, registry.oracle.dist, 'fp.esm.js')
let foundDifference = false

for (const port of selectedPorts) {
  const candidate = resolve(compareRoot, port.dist, 'fp.esm.js')
  const outputDirectory = resolve(compareRoot, 'results', port.id, options.outputName)
  await mkdir(outputDirectory, { recursive: true })

  const server = await startServer({
    '/index.html': {
      body: '<!doctype html><meta charset="utf-8"><title>feature parity</title><body></body>',
    },
    '/feature-parity-contract.js': { body: contract },
    '/upstream.js': { file: upstream },
    '/candidate.js': { file: candidate },
  })

  try {
    for (const browser of options.browsers) {
      const driver = await createDriver(browser, oracleRoot)
      try {
        const upstreamCapture = await capture(driver, server.origin, 'upstream')
        const candidateCapture = await capture(driver, server.origin, 'candidate')
        const comparison = compareContracts([upstreamCapture], [candidateCapture])
        const result = {
          schema: 1,
          generatedAt: new Date().toISOString(),
          port: {
            id: port.id,
            label: port.label,
            family: port.family,
            profile: port.profile,
            dist: port.dist,
          },
          browser: await browserIdentity(driver),
          artifacts: { upstream, candidate },
          comparison,
          upstream: upstreamCapture,
          candidate: candidateCapture,
        }
        await writeFile(
          resolve(outputDirectory, `${browser}.json`),
          `${JSON.stringify(result, null, 2)}\n`,
        )
        console.log(`${port.id} ${browser}: ${comparison.differences.length} extended difference(s)`)
        if (comparison.differences.length) foundDifference = true
      } finally {
        await driver.quit()
      }
    }
  } finally {
    await server.close()
  }
}

if (options.failOnDifference && foundDifference) process.exitCode = 2

async function capture(driver, origin, moduleName) {
  await driver.get(`${origin}/index.html`)
  const loadResult = await driver.executeAsyncScript(`
    const done = arguments[arguments.length - 1];
    const script = document.createElement('script');
    script.src = ${JSON.stringify(origin + '/feature-parity-contract.js')};
    script.onload = () => done({ ok: true });
    script.onerror = () => done({ ok: false, error: 'contract script failed to load' });
    document.head.appendChild(script);
  `)
  if (!loadResult?.ok) throw new Error(loadResult?.error ?? 'feature parity contract failed to load')
  const result = await driver.executeAsyncScript(`
    const done = arguments[arguments.length - 1];
    captureFeatureParity(${JSON.stringify(origin + '/')} + ${JSON.stringify(moduleName)} + '.js')
      .then(done, (error) => done({
        __driverError: String(error),
        stack: error && error.stack,
      }));
  `)
  if (result?.__driverError) {
    throw new Error(`${moduleName} capture failed: ${result.__driverError}\n${result.stack ?? ''}`)
  }
  return result
}

function parseArgs(args) {
  const result = {
    ports: [],
    browsers: ['chrome', 'firefox', 'safari'],
    outputName: 'feature-parity-extended',
    allVariants: false,
    failOnDifference: false,
  }
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]
    if (value === '--port') result.ports.push(args[++index])
    else if (value === '--browsers') result.browsers = args[++index].split(',').filter(Boolean)
    else if (value === '--output-name') result.outputName = args[++index]
    else if (value === '--all-variants') result.allVariants = true
    else if (value === '--fail-on-difference') result.failOnDifference = true
    else throw new Error(`unknown argument: ${value}`)
  }
  return result
}
