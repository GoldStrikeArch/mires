#!/usr/bin/env node
// Kotlin → JavaScript through the Kotlin/JS Gradle plugin: the production lane of
// experiments/fingerprintjs-kotlin/tools/build.mjs (`kotlin-production`): Gradle 9.4.1, Kotlin
// Gradle plugin 2.4.10, `compileProductionExecutableKotlinJs --no-daemon`, whole-program ES module
// output, ES2015 target, production DCE and member minimisation; then the comparator's production
// Terser pass ({ comments: false, safari10, module, compress.passes: 2 }). The Kotlin Toolchain
// js/app lane (KOTLIN_BIN) is the comparator's readable/proxy variant and is not built here.
//
// One shared Gradle project (languages/kotlin/gradle-template) is instantiated per port in its
// work directory, with `kotlin.srcDir` pointing at a fresh copy of the port's src/ and port.json
// `dependencies` (Gradle coordinates) added to jsMain. `run` is a native ES module export through
// @JsExport, so the emit is used as it is.
//
// Gradle itself is the copy the comparator build downloaded and verified against its published
// SHA-256 (experiments/fingerprintjs-kotlin/build/toolchains/gradle-9.4.1); GRADLE_BIN overrides it.
//
// Reproducibility: with the comparator's settings the production emit is not byte-reproducible.
// The IR compiler orders the supertypes of initMetadataForClass(...) by iterating identity-hashed
// sets, and HotSpot's default identity hashes differ from run to run (clean builds of the pilot:
// 3 distinct outputs in 4; of the FingerprintJS Kotlin sources: 2 in 3; daemon or in-process
// alike). The template therefore runs the compiler in the Gradle JVM (`kotlin.compiler.execution.
// strategy=in-process`) with a constant identity hash (`-XX:+UnlockExperimentalVMOptions
// -XX:hashCode=2`), which makes that iteration the insertion order: 8/8 clean pilot builds and
// 4/4 FingerprintJS builds were byte-identical, at about +1 to 2 s of compile time on the larger
// program. It is a JVM setting of the compiling process: no compiler flag and no emit patch.

import {
  existsSync, harnessRoot, main, mkdirSync, openPort, packageVersion, readText, removeFile, repositoryRoot, resolve, run,
  stageSources, stripSourceMapComment, terser, commonTerserOptions, versionOf, writeBuildInfo, writeDist, writeFileSync,
} from '../common.mjs'

const GRADLE_VERSION = '9.4.1'
const KOTLIN_PLUGIN = '2.4.10'
const TASK = 'compileProductionExecutableKotlinJs'

await main(async () => {
  const port = openPort('kotlin')
  const gradle = process.env.GRADLE_BIN
    || resolve(repositoryRoot, `experiments/fingerprintjs-kotlin/build/toolchains/gradle-${GRADLE_VERSION}/bin/gradle`)
  if (!existsSync(gradle)) {
    throw new Error(`Gradle ${GRADLE_VERSION} not found at ${gradle}; run experiments/fingerprintjs-kotlin/tools/build.mjs once (it downloads and verifies it) or set GRADLE_BIN`)
  }
  const stage = stageSources(port)
  const project = resolve(port.workDir, 'gradle')
  mkdirSync(project, { recursive: true })
  const name = `matrix-${port.library}-kotlin`
  const template = resolve(harnessRoot, 'languages/kotlin/gradle-template')
  const dependencies = (port.manifest.dependencies || []).map((coordinate) => `                implementation("${coordinate}")`).join('\n')
  writeFileSync(resolve(project, 'settings.gradle.kts'), readText(resolve(template, 'settings.gradle.kts')).replace('@@PROJECT@@', name))
  writeFileSync(resolve(project, 'gradle.properties'), readText(resolve(template, 'gradle.properties')))
  writeFileSync(resolve(project, 'build.gradle.kts'), readText(resolve(template, 'build.gradle.kts'))
    .replace('@@SOURCES@@', resolve(stage, 'src'))
    .replace('@@DEPENDENCIES@@', dependencies))

  const output = resolve(project, `build/js/packages/${name}/kotlin/${name}.mjs`)
  removeFile(output)
  // `--rerun-tasks`: every build is a fresh compile. Without it Gradle reports the Kotlin/JS compile
  // UP-TO-DATE from its own state (even after the output above is deleted), so an unchanged rebuild
  // would reuse the previous emit, and "reproduced" would prove nothing.
  run(gradle, ['-p', project, TASK, '--no-daemon', '--console=plain', '--rerun-tasks'], { cwd: project })
  if (!existsSync(output)) throw new Error(`Gradle Kotlin/JS output missing: ${output}`)
  const program = stripSourceMapComment(readText(output))
  const terserOptions = commonTerserOptions({ passes: 2 })
  const min = await terser(program, terserOptions)
  await writeDist(port, {
    min,
    readable: program,
    meta: {
      family: 'kotlin',
      variant: 'production (Gradle Kotlin/JS)',
      toolchain: {
        gradle: GRADLE_VERSION,
        kotlinGradlePlugin: KOTLIN_PLUGIN,
        java: versionOf('java', ['-version']),
        terser: packageVersion('terser'),
      },
      settings: {
        task: TASK,
        outputGranularity: 'whole-program',
        moduleKind: 'es',
        target: 'es2015',
        dce: true,
        minimizedMemberNames: true,
        compilerExecution: 'in-process (kotlin.compiler.execution.strategy)',
        compilerJvm: '-Xmx4g -XX:+UnlockExperimentalVMOptions -XX:hashCode=2 (constant identity hash: reproducible supertype order)',
        packaging: 'native ES module exports (@JsExport); comparator production Terser pass (2 passes)',
        terser: terserOptions,
      },
      attribution: { unit: 'Gradle output bytes before Terser', program: Buffer.byteLength(program), runtime: null, runtimeSeparable: false },
    },
  })
  writeBuildInfo(port, { output, gradle })
})
