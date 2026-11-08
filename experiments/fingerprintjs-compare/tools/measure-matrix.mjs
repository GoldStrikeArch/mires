#!/usr/bin/env node

import { existsSync } from 'node:fs'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'

import { fileRecord, stableJson } from '../../fingerprintjs-rust/tools/common.mjs'
import { artifactPaths, compareRoot, loadRegistry, parseArgs } from './ports.mjs'

const options = parseArgs()
const registry = await loadRegistry()
const allPorts = [registry.oracle, ...registry.ports]
const selected = options.port
  ? allPorts.filter((port) => port.id === options.port)
  : options.profile
    ? allPorts.filter((port) => port.profile === String(options.profile) || port.id === 'oracle')
    : allPorts

const matrix = {
  schema: 2,
  generatedAt: new Date().toISOString(),
  implementations: [],
}

for (const port of selected) {
  const dist = resolve(compareRoot, port.dist)
  const files = artifactPaths(dist)
  const present = Object.fromEntries(
    Object.entries(files).map(([name, path]) => [name, existsSync(path)]),
  )
  const artifacts = {}
  for (const [name, path] of Object.entries(files)) {
    if (!present[name] || name === 'attribution' || name === 'variant') continue
    try {
      artifacts[name] = { path, ...await fileRecord(path) }
    } catch (error) {
      artifacts[name] = { path, error: String(error) }
    }
  }
  let variant = null
  if (present.variant) variant = JSON.parse(await readFile(files.variant, 'utf8'))
  let attribution = null
  if (present.attribution) {
    attribution = JSON.parse(await readFile(files.attribution, 'utf8'))
  } else {
    attribution = inferAttribution(port, artifacts)
  }
  matrix.implementations.push({
    id: port.id,
    label: port.label,
    kind: port.kind,
    family: port.family || port.kind,
    profile: port.profile || variant?.profile || 'unspecified',
    runtimeDefault: Boolean(port.runtimeDefault),
    sizeDefault: Boolean(port.sizeDefault),
    frozen: port.frozen,
    dir: resolve(compareRoot, port.dir),
    dist,
    present,
    complete: present.esm && present.esmMin && present.iifeMin && present.types,
    artifacts,
    attribution,
    variant,
  })
}

const destination = resolve(compareRoot, options.output || 'matrix.json')
await mkdir(dirname(destination), { recursive: true })
await writeFile(destination, stableJson(matrix))
process.stdout.write(stableJson(matrix))

function inferAttribution(port, artifacts) {
  const attribution = {
    program: null,
    runtime: null,
    facade: null,
    jsKernels: null,
    source: 'inferred',
  }
  if (artifacts.rawProgram) attribution.program = artifacts.rawProgram.bytes
  if (artifacts.rawShim) attribution.runtime = artifacts.rawShim.bytes
  if (artifacts.rawFacade) attribution.facade = artifacts.rawFacade.bytes
  if (port.kind === 'oracle' && artifacts.esm) {
    attribution.program = artifacts.esm.bytes
    attribution.runtime = 0
    attribution.source = 'oracle-esm'
  }
  return attribution
}
