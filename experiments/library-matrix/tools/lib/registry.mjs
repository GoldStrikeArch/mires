// Registry, paths, options and receipts shared by the matrix tools (Node only).

import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, readdirSync, readlinkSync, realpathSync, statSync, writeFileSync } from 'node:fs'
import { dirname, isAbsolute, join, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
export const repositoryRoot = resolve(harnessRoot, '../..')

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

export function repoPath(path) {
  return relative(repositoryRoot, path)
}

export function stableJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`
}

export function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, stableJson(value))
}

export function readJson(path) {
  return JSON.parse(readFileSync(path, 'utf8'))
}

export function fileRecord(path) {
  const bytes = readFileSync(path)
  return { path: repoPath(path), bytes: bytes.length, sha256: sha256(bytes) }
}

export function walkFiles(dir, { skip = () => false } = {}, acc = []) {
  if (!existsSync(dir)) return acc
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const path = join(dir, entry.name)
    if (skip(path, entry)) continue
    if (entry.isDirectory()) walkFiles(path, { skip }, acc)
    else if (entry.isFile()) acc.push(path)
  }
  return acc
}

// --name value, --name=value and bare --flag; a usage error names the tool.
export function parseOptions(argv, { tool, usage, known }) {
  const values = {}
  for (let index = 0; index < argv.length; index++) {
    const value = argv[index]
    if (!value.startsWith('--')) usageError(tool, usage, `unexpected argument: ${value}`)
    const [name, inline] = value.slice(2).split('=', 2)
    if (!known.includes(name)) usageError(tool, usage, `unknown option --${name}`)
    if (inline !== undefined) values[name] = inline
    else if (argv[index + 1] !== undefined && !argv[index + 1].startsWith('--')) values[name] = argv[++index]
    else values[name] = true
  }
  return values
}

export function usageError(tool, usage, message) {
  process.stderr.write(`${tool}: ${message}\n${usage}\n`)
  process.exit(2)
}

export function integerOption(options, name, fallback, min, fail) {
  const raw = options[name]
  if (raw === undefined) return fallback
  const value = typeof raw === 'string' && /^[+-]?\d+$/.test(raw) ? Number(raw) : NaN
  if (!Number.isSafeInteger(value) || value < min) fail(`--${name} must be an integer >= ${min} (got ${raw === true ? 'no value' : JSON.stringify(raw)})`)
  return value
}

export function numberOption(options, name, fallback, fail) {
  const raw = options[name]
  if (raw === undefined) return fallback
  const value = typeof raw === 'string' ? Number(raw) : NaN
  if (!Number.isFinite(value) || value <= 0) fail(`--${name} must be a positive number (got ${raw === true ? 'no value' : JSON.stringify(raw)})`)
  return value
}

// The registry (matrix.json, or --matrix <path>) with every path resolved.
export function loadRegistry(path = resolve(harnessRoot, 'matrix.json')) {
  const registryPath = resolve(path)
  const raw = readJson(registryPath)
  if (raw.schema !== 1) throw new Error(`${registryPath}: unsupported registry schema ${raw.schema}`)
  const base = harnessRoot
  const at = (value) => (isAbsolute(value) ? value : resolve(base, value))
  const languages = {}
  for (const [id, language] of Object.entries(raw.languages)) {
    for (const field of ['label', 'builder', 'production', 'sources']) {
      if (!language[field]) throw new Error(`${registryPath}: language ${id} has no ${field}`)
    }
    languages[id] = { id, ...language, builderPath: at(language.builder) }
  }
  const libraries = {}
  for (const [id, library] of Object.entries(raw.libraries)) {
    for (const field of ['label', 'reference', 'corpus', 'output', 'environments']) {
      if (library[field] === undefined) throw new Error(`${registryPath}: library ${id} has no ${field}`)
    }
    if (!['string', 'bytes', 'json'].includes(library.output.form)) throw new Error(`${registryPath}: library ${id} output form ${library.output.form}`)
    for (const environment of library.environments) {
      if (!raw.environments[environment]) throw new Error(`${registryPath}: library ${id} names unknown environment ${environment}`)
    }
    if (!library.npm && !library.package?.standIn) throw new Error(`${registryPath}: library ${id} needs npm { package, version, entry } or a stand-in package`)
    libraries[id] = {
      id,
      ...library,
      referencePath: at(library.reference),
      corpusPath: at(library.corpus),
      contractPath: library.contract ? at(library.contract) : null,
      normalisationPath: library.normalisation ? at(library.normalisation.module) : null,
      benchVariantPath: library.bench?.variant ? at(library.bench.variant) : null,
    }
  }
  const ports = []
  for (const [libraryId, entries] of Object.entries(raw.ports)) {
    if (!libraries[libraryId]) throw new Error(`${registryPath}: ports for unknown library ${libraryId}`)
    for (const entry of entries) {
      const port = typeof entry === 'string' ? { id: entry, language: entry } : { ...entry }
      if (!languages[port.language]) throw new Error(`${registryPath}: port ${libraryId}/${port.id} has unknown language ${port.language}`)
      port.library = libraryId
      port.dir = port.dir ? at(port.dir) : resolve(harnessRoot, 'ports', libraryId, port.id)
      port.key = `${libraryId}--${port.id}`
      ports.push(port)
    }
  }
  return { path: registryPath, sha256: sha256(readFileSync(registryPath)), environments: raw.environments, languages, libraries, ports }
}

// `--library a,b|all` and `--language x,y|all` over the registry's ports.
export function selectPorts(registry, { library = 'all', language = 'all' } = {}, fail) {
  const pick = (value, known, label) => {
    if (value === true) fail(`--${label} needs a value`)
    if (value === 'all') return null
    const names = String(value).split(',').map((name) => name.trim()).filter(Boolean)
    for (const name of names) if (!known.includes(name)) fail(`unknown ${label}: ${name} (known: ${known.join(', ')})`)
    return new Set(names)
  }
  const libraries = pick(library, Object.keys(registry.libraries), 'library')
  const portIds = [...new Set(registry.ports.map((port) => port.id))]
  const languages = pick(language, portIds, 'language')
  return registry.ports.filter((port) => (!libraries || libraries.has(port.library)) && (!languages || languages.has(port.id)))
}

export function receiptsRoot(options) {
  return resolve(options.receipts && options.receipts !== true ? options.receipts : resolve(repositoryRoot, 'build/library-matrix'))
}

export const receiptPaths = (root) => ({
  root,
  builds: resolve(root, 'builds'),
  logs: resolve(root, 'logs'),
  attempts: resolve(root, 'attempts'),
  previousDist: resolve(root, 'previous-dist'),
  checks: resolve(root, 'checks'),
  bench: resolve(root, 'bench'),
  sizes: resolve(root, 'sizes.json'),
  work: resolve(root, 'work'),
})

// Every input a port's build reads from this harness: the port itself (minus dist/), its
// language's builder directory, the shared builder modules, and the root package manifests
// that pin Rollup, Terser, ReScript and Elm.
export function sourceInventory(registry, port) {
  const files = {}
  const add = (path) => {
    files[repoPath(path)] = sha256(readFileSync(path))
  }
  const skipPort = (path, entry) => entry.name.startsWith('.') || (entry.isDirectory() && path === resolve(port.dir, 'dist'))
  for (const path of walkFiles(port.dir, { skip: skipPort })) add(path)
  for (const path of walkFiles(dirname(registry.languages[port.language].builderPath))) add(path)
  for (const name of ['common.mjs', 'opam.mjs']) add(resolve(harnessRoot, 'languages', name))
  for (const name of ['package.json', 'package-lock.json']) add(resolve(repositoryRoot, name))
  const digest = sha256(JSON.stringify(Object.entries(files).sort()))
  return { digest, files }
}

export function buildReceiptPath(paths, port) {
  return resolve(paths.builds, `${port.key}.json`)
}

// Whether the port's dist/ is the output of its current build receipt: the receipt says built,
// every recorded artifact still has its recorded digest, dist/ holds nothing else, and the
// port's build inputs are the ones the receipt recorded after the build.
export function freshness(registry, paths, port) {
  const receiptPath = buildReceiptPath(paths, port)
  if (!existsSync(receiptPath)) return { fresh: false, status: 'not-built', reasons: ['no build receipt'], receipt: null }
  const receipt = readJson(receiptPath)
  if (!receipt.built) return { fresh: false, status: 'build-failed', reasons: [`build receipt records a failed build (exit ${receipt.exitCode}${receipt.timedOut ? ', timed out' : ''})`], receipt }
  const reasons = []
  const recorded = new Map(receipt.artifacts.map((artifact) => [artifact.path, artifact]))
  const distDir = resolve(port.dir, 'dist')
  const present = walkFiles(distDir).map(repoPath)
  for (const path of present) {
    const artifact = recorded.get(path)
    if (!artifact) {
      reasons.push(`unrecorded file in dist: ${path}`)
      continue
    }
    const digest = sha256(readFileSync(resolve(repositoryRoot, path)))
    if (digest !== artifact.sha256) reasons.push(`artifact changed since the build: ${path}`)
  }
  for (const path of recorded.keys()) if (!present.includes(path)) reasons.push(`recorded artifact missing: ${path}`)
  const inventory = sourceInventory(registry, port)
  if (inventory.digest !== receipt.sourceAfter.digest) {
    const changed = new Set([...Object.keys(inventory.files), ...Object.keys(receipt.sourceAfter.files)])
    for (const path of changed) {
      if (inventory.files[path] !== receipt.sourceAfter.files[path]) reasons.push(`build input changed since the build: ${path}`)
    }
  }
  return { fresh: reasons.length === 0, status: reasons.length ? 'stale' : 'fresh', reasons, receipt }
}

export function artifactOf(receipt, name = 'index.min.js') {
  return receipt?.artifacts?.find((artifact) => artifact.path.endsWith(`/dist/${name}`)) ?? null
}

// The module graph a JS module reaches: static imports and re-exports, and dynamic import()s
// whose specifier is a string literal, followed transitively (relative paths only). A dynamic
// import with a computed specifier cannot be bound and is an error.
export async function moduleGraph(entry, { label = 'module' } = {}) {
  const { parse } = await import(pathToFileURL(resolve(repositoryRoot, 'node_modules/acorn/dist/acorn.mjs')))
  const modules = []
  const seen = new Set()
  const errors = []
  const visit = (path) => {
    if (seen.has(path)) return
    seen.add(path)
    if (!existsSync(path)) {
      errors.push(`missing module ${repoPath(path)}`)
      return
    }
    const source = readFileSync(path, 'utf8')
    modules.push({ ...fileRecord(path) })
    if (!/\.(m?js)$/.test(path)) return
    let tree
    try {
      tree = parse(source, { sourceType: 'module', ecmaVersion: 'latest' })
    } catch (error) {
      errors.push(`cannot parse ${repoPath(path)}: ${error.message}`)
      return
    }
    const follow = (specifier, how) => {
      if (!specifier.startsWith('./') && !specifier.startsWith('../')) {
        errors.push(`${repoPath(path)} ${how} bare specifier ${specifier}; a ${label} imports by relative path`)
        return
      }
      visit(resolve(dirname(path), specifier))
    }
    const walk = (node) => {
      if (!node || typeof node.type !== 'string') return
      if ((node.type === 'ImportDeclaration' || node.type === 'ExportNamedDeclaration' || node.type === 'ExportAllDeclaration') && node.source) {
        follow(node.source.value, 'imports')
      }
      if (node.type === 'ImportExpression') {
        if (node.source.type === 'Literal' && typeof node.source.value === 'string') follow(node.source.value, 'dynamically imports')
        else errors.push(`${repoPath(path)} has a dynamic import() with a computed specifier, which cannot be bound`)
      }
      for (const value of Object.values(node)) {
        if (Array.isArray(value)) value.forEach(walk)
        else if (value && typeof value === 'object' && typeof value.type === 'string') walk(value)
      }
    }
    walk(tree)
  }
  visit(entry)
  return { modules, errors }
}

// Digest of every file in a directory (an installed npm package, or a stand-in package), so a
// module or data file read at run time is bound even when no import names it.
// Symbolic links are followed: a link is recorded with its target, and the content it reaches is
// hashed (a linked directory is walked, once per real path, so a cycle ends).
export function directoryDigest(dir) {
  const entries = []
  const links = []
  const visited = new Set()
  const walk = (current) => {
    const real = realpathSync(current)
    if (visited.has(real)) return
    visited.add(real)
    for (const entry of readdirSync(current, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
      const path = join(current, entry.name)
      if (entry.isDirectory() && entry.name === 'node_modules') continue
      if (entry.isSymbolicLink()) {
        const target = readlinkSync(path)
        let kind = 'dangling'
        try {
          const stat = statSync(path)
          kind = stat.isDirectory() ? 'directory' : 'file'
          if (stat.isDirectory()) walk(path)
          else entries.push([relative(dir, path), sha256(readFileSync(path))])
        } catch {
          kind = 'dangling'
        }
        links.push([relative(dir, path), target, kind])
        continue
      }
      if (entry.isDirectory()) walk(path)
      else if (entry.isFile()) entries.push([relative(dir, path), sha256(readFileSync(path))])
    }
  }
  walk(dir)
  return { path: repoPath(dir), files: entries.length, links, sha256: sha256(JSON.stringify({ entries, links })) }
}

// Library reference identity: the adapter, every module it reaches (static and literal dynamic
// imports, transitively), the whole package directory, and for npm libraries the pinned and
// installed versions.
export async function referenceIdentity(library) {
  const graph = await moduleGraph(library.referencePath, { label: 'reference adapter' })
  const { modules, errors } = graph
  const identity = { adapter: fileRecord(library.referencePath), modules, errors }
  if (library.npm) {
    const modulesRoot = library.npm.modules === 'harness' ? resolve(harnessRoot, 'node_modules') : resolve(repositoryRoot, 'node_modules')
    const packageDir = resolve(modulesRoot, library.npm.package)
    const entry = resolve(packageDir, library.npm.entry)
    identity.npm = { package: library.npm.package, pinned: library.npm.version, entry: repoPath(entry) }
    try {
      identity.npm.installed = readJson(resolve(packageDir, 'package.json')).version
      identity.npm.entryRecord = fileRecord(entry)
      identity.packageDirectory = directoryDigest(packageDir)
    } catch (error) {
      errors.push(`npm package ${library.npm.package} is not installed under ${repoPath(modulesRoot)}: ${error.message}`)
    }
    if (identity.npm.installed && identity.npm.installed !== library.npm.version) {
      errors.push(`npm package ${library.npm.package} installed ${identity.npm.installed}, pinned ${library.npm.version}`)
    }
    if (!modules.some((module) => module.path === repoPath(entry))) errors.push(`the adapter does not import the package entry ${repoPath(entry)}`)
  } else {
    identity.standIn = library.package
    if (library.package?.entry) {
      const entry = resolve(harnessRoot, library.package.entry)
      identity.packageDirectory = directoryDigest(dirname(entry))
      if (!modules.some((module) => module.path === repoPath(entry))) errors.push(`the adapter does not import the stand-in entry ${library.package.entry}`)
    }
  }
  return identity
}

export function loadCorpus(library) {
  const corpus = readJson(library.corpusPath)
  if (corpus.schema !== 1 || !Array.isArray(corpus.cases)) throw new Error(`${library.corpusPath}: expected { schema: 1, cases: [...] }`)
  const ids = new Set()
  for (const item of corpus.cases) {
    if (!item.id || ids.has(item.id)) throw new Error(`${library.corpusPath}: case ids must be unique and non-empty (${item.id})`)
    ids.add(item.id)
    if (item.input === undefined) throw new Error(`${library.corpusPath}: case ${item.id} has no input`)
  }
  return { ...corpus, record: fileRecord(library.corpusPath) }
}

// What a library's check and timing receipts are valid for: the corpus, the reference (adapter,
// every module it imports, and the npm package version and entry), the normalisation module and
// the output form. A receipt whose binding digest differs from the current one is stale: it was
// measured against another corpus, reference, normalisation or output contract.
export function libraryBinding(library, identity, corpus, extra = {}) {
  const parts = {
    corpus: { path: corpus.record.path, sha256: corpus.record.sha256, cases: corpus.cases.length },
    reference: {
      adapter: identity.adapter.sha256,
      modules: identity.modules.map(({ path, sha256: digest }) => [path, digest]),
      npm: identity.npm
        ? { package: identity.npm.package, pinned: identity.npm.pinned, installed: identity.npm.installed ?? null, entry: identity.npm.entryRecord?.sha256 ?? null }
        : null,
      standIn: identity.standIn ? { name: identity.standIn.name, version: identity.standIn.version } : null,
      packageDirectory: identity.packageDirectory?.sha256 ?? null,
    },
    normalisation: library.normalisation
      ? { name: library.normalisation.name, modules: (extra.normalisation?.modules ?? []).map(({ path, sha256: digest }) => [path, digest]) }
      : null,
    output: { form: library.output.form, async: Boolean(library.output.async) },
    benchVariant: library.bench?.variant ? (extra.benchVariant?.modules ?? []).map(({ path, sha256: digest }) => [path, digest]) : null,
  }
  return { ...parts, digest: sha256(JSON.stringify(parts)) }
}

// The library's current binding, with the module graphs of its normalisation and bench-variant
// modules; their errors (a computed dynamic import, a missing module) are the identity's errors.
export async function currentBinding(library) {
  const identity = await referenceIdentity(library)
  const corpus = loadCorpus(library)
  const extra = {}
  if (library.normalisationPath) {
    extra.normalisation = await moduleGraph(library.normalisationPath, { label: 'normalisation module' })
    identity.errors.push(...extra.normalisation.errors)
  }
  if (library.benchVariantPath) {
    extra.benchVariant = await moduleGraph(library.benchVariantPath, { label: 'bench-variant module' })
    identity.errors.push(...extra.benchVariant.errors)
  }
  identity.normalisationModules = extra.normalisation?.modules ?? []
  identity.benchVariantModules = extra.benchVariant?.modules ?? []
  return { identity, corpus, binding: libraryBinding(library, identity, corpus, extra) }
}

// The harness code a check, bench or build receipt depends on, bound by content: a receipt made by
// other code is stale, whatever the protocol version string says.
export const HARNESS_CODE = {
  check: ['tools/lib/canonical.mjs', 'tools/lib/check-runner.mjs', 'tools/check.mjs', 'tools/check-cell.mjs', 'tools/lib/browser.mjs',
    'tools/lib/cells.mjs', 'tools/lib/registry.mjs'],
  bench: ['tools/lib/canonical.mjs', 'tools/lib/protocol.mjs', 'tools/bench.mjs', 'tools/bench-cell.mjs', 'tools/lib/browser.mjs',
    'tools/lib/cells.mjs', 'tools/lib/registry.mjs'],
  build: ['tools/build.mjs', 'tools/lib/registry.mjs'],
}

export function harnessIdentity(kind) {
  const files = HARNESS_CODE[kind].map((path) => [path, sha256(readFileSync(resolve(harnessRoot, path)))])
  return { kind, files, sha256: sha256(JSON.stringify(files)) }
}

// Environment-version policy: a receipt is current for the major version of Node or of the
// browser it was made in. Node's current major is this process's; a browser's is the open
// session's (bench), or the timing receipt's when the report pairs parity with timing.
export function majorVersion(identity) {
  const version = identity?.version ?? null
  const match = version === null ? null : /^v?(\d+)/.exec(String(version))
  return match ? Number(match[1]) : null
}

// Why a check receipt cannot gate timing or count in the report, or null when it is current: it
// must be a pass, for the current artifact, under the current library binding, made by the current
// check code, and (when `environmentMajor` is given) in the same major version of the environment.
export function checkReceiptProblem(cell, { artifact, binding, harness, environmentMajor = null, requirePass = true }) {
  if (!cell) return { status: 'not-checked', reason: 'no check receipt for this environment' }
  if (cell.status === 'unavailable') return { status: 'unavailable', reason: cell.reason ?? 'the environment was unavailable when checked' }
  if (!artifact || cell.artifact?.sha256 !== artifact.sha256) return { status: 'stale', reason: 'the check receipt is for another artifact than the current build' }
  if (cell.harness?.sha256 !== harness.sha256) return { status: 'stale', reason: `the check receipt was made by other check code (${cell.checkProtocol ?? 'protocol 1'}${cell.harness ? `, code ${cell.harness.sha256.slice(0, 12)}` : ''}; current ${harness.sha256.slice(0, 12)})` }
  if (environmentMajor !== null && majorVersion(cell.environmentIdentity) !== environmentMajor) {
    return { status: 'stale', reason: `the check receipt is from ${cell.environmentIdentity?.label ?? 'another version'}, not major version ${environmentMajor}` }
  }
  if (cell.binding?.digest !== binding.digest) return { status: 'stale', reason: `the check receipt predates the current ${bindingDifference(cell.binding, binding)}` }
  if (requirePass && cell.status !== 'pass') {
    return { status: 'parity-failed', reason: `parity ${cell.status}${cell.summary ? ` ${cell.summary.equal}/${cell.summary.cases}` : ''}${cell.reason ? `: ${String(cell.reason).slice(0, 200)}` : ''}` }
  }
  return null
}

// A runtime-guard violation is a property of the port, not of one environment: when the port's
// current Node check receipt records one, the port is refused timing in every environment (a
// browser cannot see an import() of a module its page already loaded; README "Runtime guard").
export function guardProblem(nodeCell, { artifact, binding, harness }) {
  if (!nodeCell || checkReceiptProblem(nodeCell, { artifact, binding, harness, requirePass: false })) return null
  const guard = nodeCell.guard ?? {}
  const seen = [...(guard.violations ?? []), ...(guard.moduleLoads ?? []).map((load) => `module load ${load.specifier}`), ...(guard.unexpectedRequests ?? [])]
  return seen.length ? { status: 'runtime-guard', reason: `the Node check saw ${[...new Set(seen)].join('; ').slice(0, 300)}` } : null
}

export function bindingDifference(recorded, current) {
  if (!recorded) return 'library binding (the receipt records none)'
  const changed = ['corpus', 'reference', 'normalisation', 'output', 'benchVariant'].filter((key) => JSON.stringify(recorded[key]) !== JSON.stringify(current[key]))
  return changed.length ? changed.join(', ') : 'library binding'
}
