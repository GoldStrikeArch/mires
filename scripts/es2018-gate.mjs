#!/usr/bin/env node

import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

const PINNED_ACORN = '8.15.0'
const RUNTIME_NAMES = [
  'AbortController',
  'AggregateError',
  'BigInt',
  'FinalizationRegistry',
  'MessageChannel',
  'Promise',
  'TextDecoder',
  'TextEncoder',
  'WeakRef',
  'globalThis',
  'queueMicrotask',
  'structuredClone',
]

const options = parseArgs(process.argv.slice(2))
const parserModule = await import(pathToFileURL(resolve(options.parser)))
if (parserModule.version !== PINNED_ACORN) {
  throw new Error(`ES2018 parser drift: expected Acorn ${PINNED_ACORN}, found ${parserModule.version || 'unknown'}`)
}

const records = []
for (const input of options.inputs) records.push(await inspect(input))
records.sort((left, right) => left.path.localeCompare(right.path))

const featureCounts = sumMaps(records.map((record) => record.postEs2018Features))
const runtimeCounts = sumMaps(records.map((record) => record.runtimeRequirements))
const receipt = {
  schema: 2,
  target: 'ES2018',
  parser: { name: 'acorn', version: parserModule.version, pinned: PINNED_ACORN },
  syntaxOnly: true,
  strictRuntimeCompatibilityClaimed: false,
  opaqueInputPolicy: { rawJavaScript: 'validated-by-pinned-final-artifact-parser' },
  selectedPostEs2018SyntaxCapabilities: [],
  postEs2018Features: featureCounts,
  runtimeInventoryKind: 'selected-global-identifier-counts',
  runtimeRequirements: runtimeCounts,
  passed: records.every((record) => record.es2018),
  files: records,
}

const rendered = `${JSON.stringify(receipt, null, 2)}\n`
if (options.output) {
  await mkdir(dirname(resolve(options.output)), { recursive: true })
  await writeFile(resolve(options.output), rendered)
}
process.stdout.write(rendered)
if (!receipt.passed) process.exitCode = 1

async function inspect(input) {
  const path = resolve(input.path)
  const source = await readFile(path, 'utf8')
  let ast = null
  let es2018Error = null
  try {
    ast = parserModule.parse(source, parserOptions(2018, input.sourceType))
  } catch (error) {
    es2018Error = errorRecord(error, source)
    try {
      ast = parserModule.parse(source, parserOptions('latest', input.sourceType))
    } catch (latestError) {
      return {
        path: displayPath(path),
        sourceType: input.sourceType,
        bytes: Buffer.byteLength(source),
        sha256: sha256(source),
        es2018: false,
        observedMinimum: null,
        es2018Error,
        latestError: errorRecord(latestError, source),
        postEs2018Features: {},
        runtimeRequirements: {},
      }
    }
  }

  const analysis = analyze(ast)
  return {
    path: displayPath(path),
    sourceType: input.sourceType,
    bytes: Buffer.byteLength(source),
    sha256: sha256(source),
    es2018: es2018Error === null,
    observedMinimum: minimumVersion(source, input.sourceType, es2018Error === null),
    es2018Error,
    postEs2018Features: analysis.features,
    runtimeRequirements: analysis.runtime,
  }
}

function parserOptions(ecmaVersion, sourceType) {
  return { allowHashBang: true, ecmaVersion, locations: true, sourceType }
}

function minimumVersion(source, sourceType, passed2018) {
  const versions = passed2018
    ? [2015, 2016, 2017, 2018]
    : [2019, 2020, 2021, 2022, 2023, 2024, 2025]
  for (const version of versions) {
    try {
      parserModule.parse(source, parserOptions(version, sourceType))
      return version
    } catch {
      // Continue to the next published grammar level.
    }
  }
  return 'latest'
}

function analyze(ast) {
  const features = {}
  const runtime = {}
  walk(ast, null, null, (node, parent, key) => {
    if (node.type === 'Literal' && node.bigint !== undefined) increment(features, 'BigIntLiteral')
    if (node.type === 'AssignmentExpression' && ['&&=', '||=', '??='].includes(node.operator)) {
      increment(features, `LogicalAssignment:${node.operator}`)
    }
    if (node.type === 'LogicalExpression' && node.operator === '??') increment(features, 'NullishCoalescing')
    if (node.type === 'ChainExpression') increment(features, 'OptionalChaining')
    if (node.type === 'ImportExpression') increment(features, 'DynamicImport')
    if (node.type === 'PropertyDefinition') increment(features, 'ClassField')
    if (node.type === 'StaticBlock') increment(features, 'ClassStaticBlock')
    if (node.type === 'PrivateIdentifier') increment(features, 'PrivateIdentifier')
    if (node.type === 'Identifier' && RUNTIME_NAMES.includes(node.name) && isReference(parent, key)) {
      increment(runtime, node.name)
    }
  })
  return { features, runtime }
}

function walk(value, parent, key, visit) {
  if (Array.isArray(value)) {
    for (const child of value) walk(child, parent, key, visit)
    return
  }
  if (!value || typeof value !== 'object') return
  if (typeof value.type === 'string') visit(value, parent, key)
  for (const [childKey, child] of Object.entries(value)) {
    if (['end', 'loc', 'range', 'start'].includes(childKey)) continue
    walk(child, value, childKey, visit)
  }
}

function isReference(parent, key) {
  if (!parent) return true
  if ((parent.type === 'MemberExpression' || parent.type === 'Property' || parent.type === 'MethodDefinition')
      && key === 'property' && !parent.computed) return false
  if ((parent.type === 'Property' || parent.type === 'MethodDefinition') && key === 'key' && !parent.computed) return false
  if (parent.type === 'VariableDeclarator' && key === 'id') return false
  if ((parent.type === 'FunctionDeclaration' || parent.type === 'FunctionExpression') && key === 'id') return false
  if (parent.type.startsWith('Import')) return false
  return true
}

function errorRecord(error, source) {
  const offset = Number.isInteger(error.pos) ? error.pos : null
  return {
    message: String(error.message),
    offset,
    line: error.loc?.line ?? null,
    column: error.loc?.column ?? null,
    context: offset === null ? null : source.slice(Math.max(0, offset - 32), Math.min(source.length, offset + 32)),
  }
}

function increment(record, key, amount = 1) {
  record[key] = (record[key] || 0) + amount
}

function sumMaps(maps) {
  const result = {}
  for (const map of maps) for (const [key, count] of Object.entries(map)) increment(result, key, count)
  return Object.fromEntries(Object.entries(result).sort(([left], [right]) => left.localeCompare(right)))
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

function displayPath(path) {
  const shown = relative(options.root, path)
  return shown && !shown.startsWith('..') ? shown : path
}

function parseArgs(argv) {
  const parsed = { inputs: [], output: null, parser: null, root: process.cwd() }
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index]
    if (option === '--parser' || option === '--output' || option === '--root') {
      const value = argv[++index]
      if (!value) throw new Error(`${option} needs a value`)
      parsed[option.slice(2)] = resolve(value)
    } else if (option === '--module' || option === '--script') {
      const path = argv[++index]
      if (!path) throw new Error(`${option} needs a file`)
      parsed.inputs.push({ path, sourceType: option.slice(2) })
    } else {
      throw new Error(`unknown option: ${option}`)
    }
  }
  if (!parsed.parser) throw new Error('--parser is required')
  if (parsed.inputs.length === 0) throw new Error('at least one --module or --script file is required')
  return parsed
}
