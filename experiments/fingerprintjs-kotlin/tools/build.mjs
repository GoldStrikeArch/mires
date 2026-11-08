#!/usr/bin/env node

import { copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { spawnSync } from 'node:child_process'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const compareRoot = resolve(root, '../fingerprintjs-compare')
const dist = resolve(root, 'dist')
const rawDir = resolve(dist, 'raw')
const kotlinBin = process.env.KOTLIN_BIN || resolve(root, '../../experiments/tools/kotlin')
const gradleVersion = '9.4.1'
const kotlinGradleVersion = '2.4.10'
const gradleProject = resolve(root, 'gradle-production')

const NAMED = [
  'componentsToDebugString',
  'getFullscreenElement',
  'getUnstableAudioFingerprint',
  'getUnstableCanvasFingerprint',
  'getUnstableHardwareConcurrency',
  'getUnstableScreenFrame',
  'getUnstableScreenResolution',
  'getWebGLContext',
  'hashComponents',
  'isAndroid',
  'isChromium',
  'isDesktopWebKit',
  'isEdgeHTML',
  'isGecko',
  'isSamsungInternet',
  'isTrident',
  'isWebKit',
  'load',
  'loadSources',
  'murmurX64Hash128',
  'prepareForSources',
  'sources',
  'transformSource',
  'withIframe',
]

function command(cmd, args, options = {}) {
  const result = spawnSync(cmd, args, { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024, ...options })
  if (!options.quiet && result.stdout) process.stdout.write(result.stdout)
  if (!options.quiet && result.stderr) process.stderr.write(result.stderr)
  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(' ')} failed:\n${result.stderr || result.stdout}`)
  }
  return `${result.stdout || ''}\n${result.stderr || ''}`
}

async function ensureGradle() {
  if (process.env.GRADLE_BIN) return process.env.GRADLE_BIN
  const toolchains = resolve(root, 'build/toolchains')
  const home = resolve(toolchains, `gradle-${gradleVersion}`)
  const binary = resolve(home, 'bin/gradle')
  if (existsSync(binary)) return binary
  await mkdir(toolchains, { recursive: true })
  const archive = resolve(toolchains, `gradle-${gradleVersion}-bin.zip`)
  command('curl', [
    '-L', '--fail', '--retry', '3',
    `https://services.gradle.org/distributions/gradle-${gradleVersion}-bin.zip`,
    '-o', archive,
  ])
  const expected = command('curl', ['-fsSL', '--retry', '3', `https://services.gradle.org/distributions/gradle-${gradleVersion}-bin.zip.sha256`]).trim()
  const { createHash } = await import('node:crypto')
  const actual = createHash('sha256').update(await readFile(archive)).digest('hex')
  if (!/^[0-9a-f]{64}$/.test(expected) || actual !== expected) throw new Error('Gradle archive checksum mismatch')
  await writeFile(`${archive}.sha256`, `${expected}\n`)
  command('unzip', ['-q', archive, '-d', toolchains])
  if (!existsSync(binary)) throw new Error(`Gradle ${gradleVersion} did not install at ${binary}`)
  return binary
}

async function findLinkJsEntry() {
  // Toolchain 0.12 publishes artifacts separately from intermediate task outputs.
  const artifacts = resolve(root, 'build/artifacts')
  if (existsSync(artifacts)) {
    const name = `${root.split('/').pop()}.mjs`
    const candidates = (await collectFiles(artifacts)).filter(file =>
      file.endsWith(`/kotlin-output/${name}`) && file.includes('jsrelease/'))
    if (candidates.length === 1) return candidates[0]
    if (candidates.length > 1) throw new Error(`ambiguous Kotlin release entry: ${candidates}`)
  }
  // Preserve support for the archived 0.11 task layout.
  const tasks = resolve(root, 'build/tasks')
  if (!existsSync(tasks)) throw new Error(`kotlin tasks dir missing: ${tasks}`)
  const names = await readdir(tasks)
  for (const dir of names.filter(name => name.includes('linkJs') && !name.includes('Test'))) {
    const folder = join(tasks, dir)
    const files = await readdir(folder)
    const entry = files.find(name => name.endsWith('.mjs'))
    if (entry) return join(folder, entry)
  }
  throw new Error(`no Kotlin application entry in ${artifacts} or ${tasks}`)
}

async function collectFiles(dir, acc = []) {
  const entries = await readdir(dir, { withFileTypes: true })
  for (const entry of entries) {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) await collectFiles(path, acc)
    else acc.push(path)
  }
  return acc
}

function stripMap(source) {
  return source.replace(/\n\/\/# sourceMappingURL=[^\n]*\s*$/, '\n')
}

function wrapProgram(program) {
  return `(function () {\n${program}\n})();`
}

function wrapEsm(program) {
  const named = NAMED.map((name) => `export const ${name} = __exp.${name};`).join('\n')
  return `${wrapProgram(program)}
const __exp = globalThis.__fp_kotlin_exports;
delete globalThis.__fp_kotlin_exports;
${named}
export default { load, hashComponents, componentsToDebugString };
`
}

function wrapIife(program) {
  return `(function (globalThis) {
${wrapProgram(program)}
const __exp = globalThis.__fp_kotlin_exports;
delete globalThis.__fp_kotlin_exports;
const FingerprintJS = {
${NAMED.map((name) => `  ${name}: __exp.${name},`).join('\n')}
};
FingerprintJS.default = { load: FingerprintJS.load, hashComponents: FingerprintJS.hashComponents, componentsToDebugString: FingerprintJS.componentsToDebugString };
globalThis.FingerprintJS = FingerprintJS;
})(typeof globalThis !== "undefined" ? globalThis : this);
`
}

async function minify(code, format, passes) {
  const { minify } = await import(pathToFileURL(resolve(oracleModules, 'terser/main.js')))
  const result = await minify(code, {
    format: { comments: false },
    safari10: true,
    module: format === 'esm',
    compress: passes > 1 ? { passes } : {},
  })
  if (result.error) throw result.error
  if (!result.code) throw new Error(`Terser produced no ${format} output`)
  return result.code
}

async function bundleProgram(entry, treeshake) {
  const { rollup } = await import(pathToFileURL(resolve(oracleModules, 'rollup/dist/es/rollup.js')))
  const bundle = await rollup({
    input: entry,
    treeshake,
    onwarn(warning, warn) {
      if (warning.code === 'CIRCULAR_DEPENDENCY') return
      warn(warning)
    },
  })
  const generated = await bundle.generate({ format: 'es', exports: 'named' })
  await bundle.close()
  const program = generated.output.find((output) => output.type === 'chunk')?.code
  if (!program) throw new Error('Rollup produced no Kotlin program chunk')
  return program
}

async function packageVariant({ id, profile, program, passes, metadata, attribution, makeDefault = false }) {
  const out = resolve(dist, id)
  const raw = resolve(out, 'raw')
  await rm(out, { recursive: true, force: true })
  await mkdir(raw, { recursive: true })

  const esmBody = wrapEsm(program)
  const esm = `${banner}\n${esmBody}\n`
  const esmMin = await minify(esmBody, 'esm', passes)
  const iifeMin = await minify(wrapIife(program), 'iife', passes)

  await Promise.all([
    writeFile(resolve(out, 'fp.esm.js'), esm),
    writeFile(resolve(out, 'fp.esm.min.js'), `${banner}\n${esmMin}\n`),
    writeFile(resolve(out, 'fp.min.js'), `${banner}\n${iifeMin}\n`),
    writeFile(resolve(raw, 'program.js'), program),
    writeFile(resolve(out, 'variant.json'), `${JSON.stringify({
      schema: 1,
      family: 'kotlin',
      id,
      profile,
      ...metadata,
    }, null, 2)}\n`),
    writeFile(resolve(out, 'attribution.json'), `${JSON.stringify(attribution, null, 2)}\n`),
  ])
  await copyFile(dts, resolve(out, 'fp.d.ts'))

  if (makeDefault) {
    await Promise.all([
      copyFile(resolve(out, 'fp.esm.js'), resolve(dist, 'fp.esm.js')),
      copyFile(resolve(out, 'fp.esm.min.js'), resolve(dist, 'fp.esm.min.js')),
      copyFile(resolve(out, 'fp.min.js'), resolve(dist, 'fp.min.js')),
      copyFile(resolve(out, 'fp.d.ts'), resolve(dist, 'fp.d.ts')),
      copyFile(resolve(out, 'variant.json'), resolve(dist, 'variant.json')),
      copyFile(resolve(out, 'attribution.json'), resolve(dist, 'attribution.json')),
    ])
  }
}

const banner = (await readFile(resolve(compareRoot, 'banner.txt'), 'utf8')).trimEnd()
const dtsCandidates = [
  resolve(oracleRoot, 'dist/fp.d.ts'),
  resolve(root, '../fingerprintjs-rust/dist-types/fp.d.ts'),
]
const dts = dtsCandidates.find((path) => existsSync(path))
if (!dts) throw new Error('oracle fp.d.ts not found')

const kotlinToolchainIdentity = command(kotlinBin, ['--version']).trim()

// Existing Kotlin Toolchain js/app lane: useful for the readable compiler-output comparison.
const buildLog = command(kotlinBin, ['build', '-p', 'js', '-v', 'release'], { cwd: root })
const releaseIgnored = /variant argument is ignored|do not support build variants/i.test(buildLog)
const entry = await findLinkJsEntry()
const linkDir = dirname(entry)
console.log(`kotlin js/app emit: ${entry}`)
const emitFiles = await collectFiles(linkDir)
let stdlibBytes = 0
let sourceProgramBytes = 0
for (const file of emitFiles) {
  const size = (await stat(file)).size
  if (file.includes('kotlin-kotlin-stdlib') || file.includes('kotlin/js/')) stdlibBytes += size
  else sourceProgramBytes += size
}
const readableProgram = await bundleProgram(entry, false)
const proxyProgram = await bundleProgram(entry, true)

// Mainstream Gradle Kotlin/JS production lane: real production DCE and member minimization.
const gradle = await ensureGradle()
command(gradle, [
  '-p', gradleProject,
  'compileProductionExecutableKotlinJs',
  '--no-daemon',
], { cwd: root })
const gradleOutput = resolve(
  gradleProject,
  'build/js/packages/fingerprintjs-kotlin-gradle/kotlin/fingerprintjs-kotlin-gradle.mjs',
)
if (!existsSync(gradleOutput)) throw new Error(`Gradle Kotlin/JS output missing: ${gradleOutput}`)
const gradleProgram = stripMap(await readFile(gradleOutput, 'utf8'))

await rm(dist, { recursive: true, force: true })
await mkdir(rawDir, { recursive: true })
await Promise.all([
  writeFile(resolve(rawDir, 'program.js'), readableProgram),
  writeFile(resolve(rawDir, 'program.proxy-production.js'), proxyProgram),
  writeFile(resolve(rawDir, 'program.gradle-production.js'), gradleProgram),
  writeFile(resolve(rawDir, 'emit-path.txt'), `${entry}\n`),
  writeFile(resolve(rawDir, 'gradle-emit-path.txt'), `${gradleOutput}\n`),
  writeFile(resolve(rawDir, 'attribution.json'), `${JSON.stringify({
    emit: entry,
    gradleEmit: gradleOutput,
    programBytes: sourceProgramBytes,
    stdlibBytes,
    totalEmitBytes: sourceProgramBytes + stdlibBytes,
    jsAppReleaseRequested: true,
    jsAppCompilerReleaseEffective: !releaseIgnored,
    files: emitFiles.map((file) => file.slice(linkDir.length + 1)),
  }, null, 2)}\n`),
])

await packageVariant({
  id: 'readable',
  profile: 'readable',
  program: readableProgram,
  passes: 1,
  makeDefault: true,
  metadata: {
    compiler: `${kotlinToolchainIdentity} js/app`,
    releaseRequested: true,
    compilerReleaseEffective: !releaseIgnored,
    releaseDiagnostic: releaseIgnored
      ? 'The js/app platform reports that build variants are unsupported.'
      : null,
    rollupTreeshake: false,
    terserPasses: 1,
  },
  attribution: {
    program: Buffer.byteLength(readableProgram),
    sourceProgramModules: sourceProgramBytes,
    runtime: stdlibBytes,
    runtimeSeparable: true,
    facade: null,
    source: 'Kotlin js/app linker modules plus Rollup',
  },
})

await packageVariant({
  id: 'proxy-production',
  profile: 'proxy',
  program: proxyProgram,
  passes: 2,
  metadata: {
    compiler: `${kotlinToolchainIdentity} js/app`,
    releaseRequested: true,
    compilerReleaseEffective: !releaseIgnored,
    releaseDiagnostic: 'Packaging proxy only: js/app ignores release; Rollup tree-shaking and two Terser passes are applied.',
    rollupTreeshake: true,
    terserPasses: 2,
  },
  attribution: {
    program: Buffer.byteLength(proxyProgram),
    sourceProgramModules: sourceProgramBytes,
    runtime: stdlibBytes,
    runtimeSeparable: true,
    facade: null,
    source: 'Kotlin js/app linker modules plus production packaging proxy',
  },
})

await packageVariant({
  id: 'production',
  profile: 'production',
  program: gradleProgram,
  passes: 2,
  metadata: {
    compiler: `Kotlin/JS Gradle plugin ${kotlinGradleVersion}`,
    gradle: gradleVersion,
    task: 'compileProductionExecutableKotlinJs',
    compilerReleaseEffective: true,
    dce: true,
    minimizedMemberNames: true,
    outputGranularity: 'whole-program',
    moduleKind: 'es',
    target: 'es2015',
    terserPasses: 2,
  },
  attribution: {
    program: Buffer.byteLength(gradleProgram),
    runtime: null,
    runtimeSeparable: false,
    facade: null,
    source: 'Kotlin/JS Gradle production whole-program output',
  },
})

console.log(`wrote ${dist}/readable, ${dist}/proxy-production, and ${dist}/production`)
console.log(`root dist remains the readable-first alias; production is Gradle Kotlin/JS ${kotlinGradleVersion}`)
console.log(`js/app source modules ${sourceProgramBytes} bytes, stdlib leftover ${stdlibBytes} bytes; js/app release effective: ${!releaseIgnored}`)
console.log(`Gradle production program ${Buffer.byteLength(gradleProgram)} bytes`)
