#!/usr/bin/env node

import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { args, fileRecord, stableJson } from '../tools/common.mjs'
import { startServer } from '../tools/static-server.mjs'
import { compareContracts } from './compare.mjs'
import { browserIdentity, createDriver } from './webdriver.mjs'

const artifactRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const options = args()
const oracleRoot = resolve(options.oracle || resolve(artifactRoot, '../fingerprintjs'))
const candidate = resolve(options.candidate || resolve(artifactRoot, 'dist/fp.esm.js'))
const upstreamFile = resolve(oracleRoot, 'dist/fp.esm.js')
const artifacts = {
  candidate: { path: candidate, ...await fileRecord(candidate) },
  upstream: { path: upstreamFile, ...await fileRecord(upstreamFile) },
}
const browsers = String(options.browsers || options.browser || 'chrome,firefox,safari').split(',')
const repetitions = Number(options.repetitions || 2)
const outputRoot = resolve(options.output || resolve(artifactRoot, 'tests/results'))
const contract = await readFile(resolve(artifactRoot, 'tests/browser/contract.js'))
const server = await startServer({
  '/index.html': { body: '<!doctype html><meta charset="utf-8"><title>Fingerprint parity</title>' },
  '/contract.js': { body: contract },
  '/upstream.js': { file: upstreamFile },
  '/candidate.js': { file: candidate },
})

let failed = false
try {
  await mkdir(outputRoot, { recursive: true })
  for (const browser of browsers) {
    const driver = await createDriver(browser, oracleRoot, { headed: options.headed === true })
    try {
      const loadPage = async () => {
        await driver.get(`${server.origin}/index.html`)
        await driver.executeAsyncScript(`
          const done = arguments[arguments.length - 1]
          const script = document.createElement('script')
          script.src = ${JSON.stringify(`${server.origin}/contract.js`)}
          script.onload = () => done()
          script.onerror = () => done({ error: 'Failed to load contract.js' })
          document.head.appendChild(script)
        `)
      }
      const upstream = []
      const candidateCaptures = []
      for (let index = 0; index < repetitions; ++index) {
        await loadPage()
        const order = index % 2 ? ['candidate', 'upstream'] : ['upstream', 'candidate']
        for (const implementation of order) {
          const capture = await driver.executeAsyncScript(`
            const done = arguments[arguments.length - 1]
            captureContract(${JSON.stringify(`${server.origin}/`) } + ${JSON.stringify(implementation)} + '.js').then(done, (error) => done({ __driverError: String(error), stack: error && error.stack }))
          `)
          ;(implementation === 'upstream' ? upstream : candidateCaptures).push(capture)
        }
      }
      await loadPage()
      const comparison = compareContracts(upstream, candidateCaptures)
      const monitoringDefault = await driver.executeAsyncScript(`
        const done = arguments[arguments.length - 1]
        captureMonitoring(${JSON.stringify(`${server.origin}/candidate.js`)}, false).then(done, (error) => done({ error: String(error) }))
      `)
      const monitoringExplicit = await driver.executeAsyncScript(`
        const done = arguments[arguments.length - 1]
        captureMonitoring(${JSON.stringify(`${server.origin}/candidate.js`)}, true).then(done, (error) => done({ error: String(error) }))
      `)
      const expectedMonitoring = [['xhr', 'https://m1.openfpcdn.io/fingerprintjs/v5.2.0/npm-monitoring']]
      const sameRequests = (actual, expected) =>
        Array.isArray(actual) &&
        actual.length === expected.length &&
        actual.every((row, index) => row[0] === expected[index][0] && row[1] === expected[index][1])
      if (!sameRequests(monitoringDefault, expectedMonitoring) || !sameRequests(monitoringExplicit, expectedMonitoring)) {
        comparison.differences.push({ path: '$.monitoring', candidate: { default: monitoringDefault, explicit: monitoringExplicit }, upstream: { default: expectedMonitoring, explicit: expectedMonitoring } })
      }
      const result = {
        browser: await browserIdentity(driver),
        candidate,
        artifacts,
        repetitions,
        comparison,
        monitoring: { default: monitoringDefault, explicit: monitoringExplicit },
        raw: options['keep-raw'] ? { upstream, candidate: candidateCaptures } : undefined,
      }
      await writeFile(resolve(outputRoot, `${browser}.json`), stableJson(result))
      console.log(`${browser}: ${comparison.differences.length} normalized difference(s)`)
      failed ||= comparison.differences.length > 0
    } finally {
      await driver.quit()
    }
  }
} finally {
  await server.close()
}

if (failed) process.exitCode = 1
