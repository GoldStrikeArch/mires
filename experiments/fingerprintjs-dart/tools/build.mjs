#!/usr/bin/env node

import { copyFile, mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { gzipSync, brotliCompressSync, constants as zlibConstants } from 'node:zlib'

const here = dirname(fileURLToPath(import.meta.url))
const root = resolve(here, '..')
const oracleRoot = resolve(root, '../fingerprintjs')
const oracleModules = resolve(root, '../../node_modules')
const dist = resolve(root, 'dist')
const rawDir = resolve(dist, 'raw')
const dart = process.env.DART || 'dart'
const bannerPath = resolve(root, '../fingerprintjs-compare/banner.txt')

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
  const result = spawnSync(cmd, args, { encoding: 'utf8', ...options })
  if (result.status !== 0) {
    throw new Error(`${cmd} ${args.join(' ')} failed:\n${result.stderr || result.stdout}`)
  }
  return result.stdout
}

function wrapProgram(program) {
  return `(function (self, window, document, navigator, screen, location) {\n${program}\n})(typeof globalThis !== "undefined" ? globalThis : this, void 0, void 0, void 0, void 0, void 0);`
}

function wrapEsm(program) {
  const named = NAMED.map((name) => `export const ${name} = __exp.${name};`).join('\n')
  return `${wrapProgram(program)}
const __exp = globalThis.__fp_dart_exports;
delete globalThis.__fp_dart_exports;
${named}
export default { load, hashComponents, componentsToDebugString };
`
}

function wrapIife(program) {
  return `(function (globalThis) {
${wrapProgram(program)}
const __exp = globalThis.__fp_dart_exports;
delete globalThis.__fp_dart_exports;
const FingerprintJS = {
${NAMED.map((name) => `  ${name}: __exp.${name},`).join('\n')}
};
FingerprintJS.default = { load: FingerprintJS.load, hashComponents: FingerprintJS.hashComponents, componentsToDebugString: FingerprintJS.componentsToDebugString };
globalThis.FingerprintJS = FingerprintJS;
})(typeof globalThis !== "undefined" ? globalThis : this);
`
}

function stripMap(source) {
  return source.replace(/\n\/\/# sourceMappingURL=[\s\S]*$/, '\n')
}

async function minify(code, format) {
  const terserPath = resolve(oracleModules, 'terser/main.js')
  const { minify } = await import(terserPath)
  const result = await minify(code, {
    format: { comments: false },
    safari10: true,
    module: format === 'esm',
  })
  if (result.error) throw result.error
  if (!result.code) throw new Error(`Terser produced no ${format} output`)
  return result.code
}

function measure(bytes) {
  return {
    bytes: bytes.length,
    gzip9: gzipSync(bytes, { level: 9 }).length,
    brotli11: brotliCompressSync(bytes, {
      params: { [zlibConstants.BROTLI_PARAM_QUALITY]: 11 },
    }).length,
    sha256: createHash('sha256').update(bytes).digest('hex'),
  }
}

async function packageVariant({ id, profile, program, compilerFlags, makeDefault = false }) {
  const out = resolve(dist, id)
  const raw = resolve(out, 'raw')
  await rm(out, { recursive: true, force: true })
  await mkdir(raw, { recursive: true })

  const esmBody = wrapEsm(program)
  const esm = `${banner}\n${esmBody}\n`
  const esmMin = await minify(esmBody, 'esm')
  const iifeMin = await minify(wrapIife(program), 'iife')

  await Promise.all([
    writeFile(resolve(out, 'fp.esm.js'), esm),
    writeFile(resolve(out, 'fp.esm.min.js'), `${banner}\n${esmMin}\n`),
    writeFile(resolve(out, 'fp.min.js'), `${banner}\n${iifeMin}\n`),
    writeFile(resolve(raw, 'program.js'), program),
    writeFile(resolve(out, 'variant.json'), `${JSON.stringify({
      schema: 1,
      family: 'dart',
      id,
      profile,
      compiler: 'dart2js',
      compilerFlags,
      packaging: 'named-export facade plus common Terser pass',
      runtimeSeparable: false,
      semanticSafety: id === 'o2' ? 'safe production optimizations' : 'default readable-first optimizations',
    }, null, 2)}\n`),
    writeFile(resolve(out, 'attribution.json'), `${JSON.stringify({
      program: Buffer.byteLength(program),
      runtime: null,
      runtimeSeparable: false,
      facade: null,
      source: 'dart2js whole-program output',
    }, null, 2)}\n`),
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

  return {
    program: measure(Buffer.from(program)),
    esm: measure(Buffer.from(esm)),
    esmMin: measure(Buffer.from(`${banner}\n${esmMin}\n`)),
    iifeMin: measure(Buffer.from(`${banner}\n${iifeMin}\n`)),
  }
}

const banner = existsSync(bannerPath)
  ? (await readFile(bannerPath, 'utf8')).trimEnd()
  : (await readFile(resolve(root, 'LICENSE'), 'utf8'))

const dtsCandidates = [
  resolve(oracleRoot, 'dist/fp.d.ts'),
  resolve(root, '../fingerprintjs-rust/dist-types/fp.d.ts'),
]
const dts = dtsCandidates.find((path) => existsSync(path))
if (!dts) throw new Error('oracle fp.d.ts not found')

command(dart, ['pub', 'get'], { cwd: root, stdio: 'inherit' })
await mkdir(resolve(root, 'build'), { recursive: true })
await mkdir(rawDir, { recursive: true })
await mkdir(dist, { recursive: true })

const o1Path = resolve(root, 'build/fp.o1.js')
const o2Path = resolve(root, 'build/fp.o2.js')
command(dart, ['compile', 'js', '-O1', '--no-source-maps', '-o', o1Path, 'web/main.dart'], {
  cwd: root,
  stdio: 'inherit',
})
command(dart, ['compile', 'js', '-O2', '--no-source-maps', '-o', o2Path, 'web/main.dart'], {
  cwd: root,
  stdio: 'inherit',
})

const programO1 = stripMap(await readFile(o1Path, 'utf8'))
const programO2 = stripMap(await readFile(o2Path, 'utf8'))
await Promise.all([
  writeFile(resolve(rawDir, 'program.o1.js'), programO1),
  writeFile(resolve(rawDir, 'program.o2.js'), programO2),
])

const variants = {
  o1: await packageVariant({
    id: 'o1',
    profile: 'readable',
    program: programO1,
    compilerFlags: ['-O1', '--no-source-maps'],
    makeDefault: true,
  }),
  o2: await packageVariant({
    id: 'o2',
    profile: 'production',
    program: programO2,
    compilerFlags: ['-O2', '--no-source-maps'],
  }),
}

await writeFile(resolve(rawDir, 'sizes.json'), `${JSON.stringify(variants, null, 2)}\n`)
console.log(`wrote ${dist}/o1 and ${dist}/o2; root dist remains the readable-first -O1 alias`)
console.log(`dart2js -O1 program ${variants.o1.program.bytes} bytes; -O2 ${variants.o2.program.bytes} bytes`)
