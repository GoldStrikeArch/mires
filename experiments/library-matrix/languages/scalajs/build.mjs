#!/usr/bin/env node
// Scala 3 → JavaScript through Scala.js, at the production settings of
// experiments/fingerprintjs-scalajs/tools/build.mjs: `scala-cli --power package --js
// --js-module-kind es --js-mode release` (fullLinkJS), Scala 3.8.4 and Scala.js 1.22.0 pinned by
// using-directives, then the common Terser pass ({ comments: false, safari10, module }). The
// output is a native ES module; `run` is exported with @JSExportTopLevel("run").
//
// scala-cli comes from PATH with the Coursier bin directory in front (as the comparator build
// sets it); COURSIER_REPOSITORIES defaults to Maven Central, as the comparator runner sets it.
// The port holds src/*.scala; the builder writes project.scala (the pins, plus port.json
// `dependencies`) next to a fresh copy of the sources.

import { homedir } from 'node:os'

import {
  existsSync, main, openPort, packageVersion, readText, resolve, run, stageSources, stripSourceMapComment, terser,
  commonTerserOptions, versionOf, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'

const SCALA = '3.8.4'
const SCALAJS = '1.22.0'

await main(async () => {
  const port = openPort('scalajs')
  const stage = stageSources(port)
  const coursierBin = resolve(homedir(), 'Library/Application Support/Coursier/bin')
  const env = {
    ...process.env,
    PATH: `${coursierBin}:${process.env.PATH || ''}`,
    COURSIER_REPOSITORIES: process.env.COURSIER_REPOSITORIES || 'https://repo.maven.apache.org/maven2',
  }
  const dependencies = (port.manifest.dependencies || []).map((dependency) => `//> using dep ${dependency}\n`).join('')
  writeFileSync(resolve(stage, 'project.scala'), `//> using scala ${SCALA}
//> using platform js
//> using jsVersion ${SCALAJS}
//> using jsModuleKind es
${dependencies}`)
  const output = resolve(port.workDir, 'release.js')
  for (const stale of [output, `${output}.map`]) if (existsSync(stale)) writeFileSync(stale, '')
  run('scala-cli', ['--power', 'package', stage, '--js', '--js-module-kind', 'es', '--js-mode', 'release', '-f', '-o', output],
    { cwd: stage, env })
  const program = stripSourceMapComment(readText(output))
  if (!program.trim()) throw new Error(`scala-cli produced no release output: ${output}`)
  const min = await terser(program, commonTerserOptions())
  await writeDist(port, {
    min,
    readable: program,
    meta: {
      family: 'scalajs',
      variant: 'full (fullLinkJS / release)',
      toolchain: {
        scalaCli: versionOf('scala-cli', ['version'], { env }),
        scala: SCALA,
        scalaJs: SCALAJS,
        terser: packageVersion('terser'),
      },
      settings: {
        command: 'scala-cli --power package --js --js-module-kind es --js-mode release',
        packaging: 'native ES module exports; common Terser pass',
        terser: commonTerserOptions(),
      },
      attribution: { unit: 'linked output bytes before Terser', program: Buffer.byteLength(program), runtime: null, runtimeSeparable: false },
    },
  })
  writeBuildInfo(port, { output })
})
