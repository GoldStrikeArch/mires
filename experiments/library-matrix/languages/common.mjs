// Shared helpers for the per-language builders in languages/<language>/build.mjs.
//
// A builder compiles ONE port directory (ports/<library>/<language>/) with its toolchain at the
// family's production settings and packages the result as dist/index.min.js (production) and
// dist/index.js (the same compile before the final minifier pass). Both must be ES modules with
// no imports and a named export `run`. Toolchain caches and every intermediate file live in the
// port's work directory under build/library-matrix/work/, never in the port itself.
//
// Usage of every builder: node languages/<language>/build.mjs --port <port dir> [--work <dir>]

import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

export const harnessRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
export const repositoryRoot = resolve(harnessRoot, '../..')
export const rootModules = resolve(repositoryRoot, 'node_modules')
export const toolchainsRoot = resolve(repositoryRoot, '.toolchains')

// The exports global that families without native ES module output (dart2js, js_of_ocaml) set
// during program start-up; the generated facade reads it once and deletes it.
export const EXPORTS_GLOBAL = '__matrixExports'

export function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

export function parseArgs(argv = process.argv.slice(2)) {
  const values = { _: [] }
  for (let index = 0; index < argv.length; index++) {
    const value = argv[index]
    if (!value.startsWith('--')) {
      values._.push(value)
      continue
    }
    const [name, inline] = value.slice(2).split('=', 2)
    if (inline !== undefined) values[name] = inline
    else if (argv[index + 1] !== undefined && !argv[index + 1].startsWith('--')) values[name] = argv[++index]
    else values[name] = true
  }
  return values
}

// Reads the port manifest and resolves the port, dist and work directories.
export function openPort(language, argv = process.argv.slice(2)) {
  const options = parseArgs(argv)
  if (!options.port || options.port === true) throw new Error(`usage: node languages/${language}/build.mjs --port <ports/<library>/${language}> [--work <dir>]`)
  const portDir = resolve(options.port)
  const manifestPath = resolve(portDir, 'port.json')
  if (!existsSync(manifestPath)) throw new Error(`port manifest missing: ${manifestPath}`)
  const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
  if (manifest.language !== language) throw new Error(`${manifestPath} declares language ${manifest.language}, not ${language}`)
  if (!manifest.library) throw new Error(`${manifestPath} declares no library`)
  const workDir = resolve(options.work && options.work !== true
    ? options.work
    : resolve(repositoryRoot, 'build/library-matrix/work', manifest.library, language))
  const distDir = resolve(portDir, 'dist')
  mkdirSync(workDir, { recursive: true })
  return { options, portDir, manifest, workDir, distDir, library: manifest.library, language }
}

// Runs a command; throws with its output on a non-zero exit. `capture` returns stdout.
export function run(command, args, { cwd, env, capture = false, input } = {}) {
  const result = spawnSync(command, args, {
    cwd,
    env: env || process.env,
    input,
    encoding: 'utf8',
    maxBuffer: 256 * 1024 * 1024,
    stdio: capture || input !== undefined ? ['pipe', 'pipe', 'pipe'] : 'inherit',
  })
  if (result.error) throw new Error(`${command} could not start: ${result.error.message}`)
  if (result.status !== 0) {
    if (capture || input !== undefined) {
      if (result.stdout) process.stdout.write(result.stdout)
      if (result.stderr) process.stderr.write(result.stderr)
    }
    throw new Error(`${command} ${args.join(' ')} failed (exit ${result.status ?? result.signal})`)
  }
  return capture || input !== undefined ? `${result.stdout ?? ''}` : ''
}

// First line of a version command's combined output, or null when the tool cannot run.
export function versionOf(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options })
  if (result.status !== 0) return null
  return `${result.stdout || ''}${result.stderr || ''}`.trim().split('\n').filter(Boolean).join(' | ') || null
}

export function packageVersion(name) {
  return JSON.parse(readFileSync(resolve(rootModules, name, 'package.json'), 'utf8')).version
}

// Replaces <work>/<subdir> with a fresh copy of the port's sources (everything except dist/ and
// the manifest), so a deleted source file can never survive in a toolchain's input tree.
export function stageSources(port, { into = 'stage', exclude = [] } = {}) {
  const target = resolve(port.workDir, into)
  rmSync(target, { recursive: true, force: true })
  mkdirSync(target, { recursive: true })
  const skip = new Set(['dist', 'port.json', ...exclude])
  for (const entry of readdirSync(port.portDir, { withFileTypes: true })) {
    if (skip.has(entry.name) || entry.name.startsWith('.')) continue
    cpSync(resolve(port.portDir, entry.name), resolve(target, entry.name), { recursive: true })
  }
  return target
}

export function listFiles(dir, accept = () => true, base = dir, acc = []) {
  if (!existsSync(dir)) return acc
  for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) listFiles(path, accept, base, acc)
    else if (entry.isFile() && accept(path)) acc.push(path)
  }
  return acc
}

export function removeFile(path) {
  rmSync(path, { force: true })
}

export function readText(path) {
  return readFileSync(path, 'utf8')
}

export function stripSourceMapComment(source) {
  return source.replace(/\n\/\/# sourceMappingURL=[^\n]*\s*$/, '\n')
}

let terserModule
export async function terser(code, options) {
  terserModule ||= await import(pathToFileURL(resolve(rootModules, 'terser/main.js')))
  // Terser writes defaults into the options object it is given; keep the caller's copy intact.
  const result = await terserModule.minify(code, structuredClone(options))
  if (!result.code) throw new Error('Terser produced no output')
  return result.code
}

// The common Terser pass of the comparator ports (oracle terser.config): no comments, safari10,
// module mode. `passes` follows the family's comparator build (Kotlin production uses 2).
export function commonTerserOptions({ passes = 1 } = {}) {
  return {
    format: { comments: false },
    safari10: true,
    module: true,
    ...(passes > 1 ? { compress: { passes } } : {}),
  }
}

let rollupModule
export async function rollupBundle(input, { plugins = [], treeshake = true } = {}) {
  rollupModule ||= await import(pathToFileURL(resolve(rootModules, 'rollup/dist/es/rollup.js')))
  const warnings = []
  const bundle = await rollupModule.rollup({
    input,
    plugins,
    treeshake,
    onwarn(warning) {
      if (warning.code === 'THIS_IS_UNDEFINED' || warning.code === 'CIRCULAR_DEPENDENCY') return
      warnings.push(`${warning.code}: ${warning.message}`)
    },
  })
  const generated = await bundle.generate({ format: 'es', exports: 'named', sourcemap: false })
  await bundle.close()
  const chunks = generated.output.filter((output) => output.type === 'chunk')
  if (chunks.length !== 1) throw new Error(`Rollup produced ${chunks.length} chunks for ${input}`)
  const chunk = chunks[0]
  const modules = Object.fromEntries(Object.entries(chunk.modules).map(([id, info]) => [id, info.renderedLength]))
  return { code: chunk.code, modules, warnings }
}

export async function nodeResolvePlugin() {
  const { createRequire } = await import('node:module')
  const require = createRequire(resolve(repositoryRoot, 'package.json'))
  const { nodeResolve } = require('@rollup/plugin-node-resolve')
  return nodeResolve({ browser: true, exportConditions: ['browser', 'import', 'default'] })
}

// Facade for families whose program publishes its exports object on globalThis.__matrixExports
// during start-up (dart2js, js_of_ocaml). `prefix` and `suffix` wrap the program text (a function
// binding environment names, for instance); the facade itself only loads and re-exports.
export function globalExportsFacade(program, { names = ['run'], prefix = '', suffix = '' } = {}) {
  return `${prefix}${program}${suffix}
const __matrix = globalThis.${EXPORTS_GLOBAL};
delete globalThis.${EXPORTS_GLOBAL};
if (!__matrix) throw new Error('program did not publish globalThis.${EXPORTS_GLOBAL}');
${names.map((name) => `export const ${name} = __matrix.${name};`).join('\n')}
`
}

let acornModule
async function acorn() {
  acornModule ||= await import(pathToFileURL(resolve(rootModules, 'acorn/dist/acorn.mjs')))
  return acornModule
}

// The port convention's shape gate: an ES module, no static or dynamic imports, no re-exports
// from another module, and a named export `run`.
export async function moduleShape(code) {
  const { parse } = await acorn()
  const tree = parse(code, { sourceType: 'module', ecmaVersion: 'latest' })
  const imports = []
  const exports = []
  const walk = (node) => {
    if (!node || typeof node.type !== 'string') return
    if (node.type === 'ImportExpression') imports.push('import()')
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) value.forEach(walk)
      else if (value && typeof value === 'object' && typeof value.type === 'string') walk(value)
    }
  }
  for (const node of tree.body) {
    if (node.type === 'ImportDeclaration') imports.push(node.source.value)
    if ((node.type === 'ExportNamedDeclaration' || node.type === 'ExportAllDeclaration') && node.source) imports.push(node.source.value)
    if (node.type === 'ExportNamedDeclaration') {
      if (node.declaration?.declarations) for (const declaration of node.declaration.declarations) exports.push(declaration.id.name)
      else if (node.declaration?.id) exports.push(node.declaration.id.name)
      for (const specifier of node.specifiers || []) exports.push(specifier.exported.name ?? specifier.exported.value)
    }
    if (node.type === 'ExportDefaultDeclaration') exports.push('default')
  }
  walk(tree)
  return { imports, exports: exports.sort(), ok: imports.length === 0 && exports.includes('run') }
}

// Writes dist/ from scratch: index.min.js (production), index.js (same compile before the final
// minifier) and meta.json (deterministic description of the build: toolchain, settings,
// attribution, runtime floor). Volatile facts (durations, absolute paths) go to the work dir's
// build-info.json instead, which the runner copies into the receipt.
export async function writeDist(port, { min, readable, meta, extra = {} }) {
  for (const [name, code] of [['index.min.js', min], ['index.js', readable]]) {
    if (code === undefined) continue
    const shape = await moduleShape(code)
    if (!shape.ok) {
      throw new Error(`${name} violates the port convention: imports [${shape.imports.join(', ')}], exports [${shape.exports.join(', ')}]`)
    }
  }
  rmSync(port.distDir, { recursive: true, force: true })
  mkdirSync(port.distDir, { recursive: true })
  writeFileSync(resolve(port.distDir, 'index.min.js'), min.endsWith('\n') ? min : `${min}\n`)
  if (readable !== undefined) writeFileSync(resolve(port.distDir, 'index.js'), readable.endsWith('\n') ? readable : `${readable}\n`)
  for (const [name, content] of Object.entries(extra)) writeFileSync(resolve(port.distDir, name), content)
  const ordered = { schema: 1, library: port.library, language: port.language, ...meta }
  writeFileSync(resolve(port.distDir, 'meta.json'), `${JSON.stringify(ordered, null, 2)}\n`)
}

export function writeBuildInfo(port, info) {
  writeFileSync(resolve(port.workDir, 'build-info.json'), `${JSON.stringify(info, null, 2)}\n`)
}

// Runs the builder body and turns any exception into a non-zero exit with a short message.
export async function main(body) {
  const started = Date.now()
  try {
    await body()
    process.stdout.write(`builder finished in ${((Date.now() - started) / 1000).toFixed(1)} s\n`)
  } catch (error) {
    process.stderr.write(`${error?.stack || error}\n`)
    process.exitCode = 1
  }
}

export { basename, copyFileSync, dirname, existsSync, join, mkdirSync, readFileSync, relative, resolve, rmSync, statSync, writeFileSync, cpSync }
