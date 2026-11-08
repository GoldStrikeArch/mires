#!/usr/bin/env node

import { existsSync } from 'node:fs'
import { readFile, writeFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const compareRoot = resolve(here, '..')
const options = parseArgs(process.argv.slice(2))
const staticReport = JSON.parse(await readFile(resolve(compareRoot, options.staticInput), 'utf8'))
const staticById = new Map(staticReport.results.map((row) => [row.id, row]))
const canonicalIds = [
  'rust-idiomatic', 'rescript', 'melange', 'jsoo', 'rust-compact',
  'rust-hybrid', 'dart-o2', 'scalajs-full', 'elm-production', 'kotlin-production',
]
const browsers = ['chrome', 'firefox', 'safari']
const rows = []

for (const id of canonicalIds) {
  const staticRow = staticById.get(id)
  if (!staticRow) throw new Error(`static audit is missing ${id}`)
  const baseline = await readPhase(id, options.baselinePhase)
  const extended = await readPhase(id, options.extendedPhase)
  const baselineDifferences = annotateDifferences(baseline)
  const extendedDifferences = annotateDifferences(extended)
  const metadataDifferences = extendedDifferences.filter((difference) => isMetadataPath(difference.path))
  const behavioralDifferences = extendedDifferences.filter((difference) => !isMetadataPath(difference.path))
  const behavioralIssues = summarizeBehavioralIssues(id, behavioralDifferences)
  const metadataSummary = summarizeMetadataIssues(staticRow, metadataDifferences)

  const coreFingerprintParity =
    staticRow.sourceRegistryExact === true &&
    staticRow.sourceMetadataDifferences.length === 0 &&
    baseline.length === browsers.length &&
    baselineDifferences.length === 0
  const upstreamSideEffectParity = extended.length === browsers.length && behavioralIssues.length === 0
  const observableFunctionMetadata = metadataSummary.differences.length === 0
  const apiKeysAndTypes = staticRow.contractApiExact === true
  const iifeApiShape = staticRow.iifeNamespaceExact === true
  const oracleFormatCoverage = staticRow.packageSurfaceComplete === true
  const strictEsmParity =
    coreFingerprintParity && upstreamSideEffectParity && observableFunctionMetadata && apiKeysAndTypes
  const artifactPackageParity = strictEsmParity && iifeApiShape && oracleFormatCoverage

  rows.push({
    id,
    label: staticRow.label,
    family: staticRow.family,
    coreFingerprintParity,
    upstreamSideEffectParity,
    observableFunctionMetadata,
    apiKeysAndTypes,
    iifeApiShape,
    oracleFormatCoverage,
    strictEsmParity,
    artifactPackageParity,
    baselineBrowsers: browserNames(baseline),
    extendedBrowsers: browserNames(extended),
    baselineDifferenceCount: baselineDifferences.length,
    behavioralIssues,
    metadataIssues: metadataSummary.differences,
    metadataSummary: {
      exportedFunctions: metadataSummary.exportedFunctions,
      returnedFunctions: metadataSummary.returnedFunctions,
    },
    behavioralDifferences: uniqueDifferences(behavioralDifferences),
    metadataDifferences: uniqueDifferences(metadataDifferences),
    missingRequiredArtifacts: Object.entries(staticReport.expected.oracleArtifacts)
      .filter(([file, required]) => required && staticRow.artifacts?.[file] !== true)
      .map(([file]) => file),
  })
}

const report = {
  schema: 2,
  generatedAt: new Date().toISOString(),
  oracleCommit: staticReport.oracleCommit,
  expected: {
    exportCount: staticReport.expected.exports.length,
    sourceCount: staticReport.expected.sources.length,
    exports: staticReport.expected.exports,
    defaultExports: staticReport.expected.defaultExports,
    sources: staticReport.expected.sources,
    declarationsSha256: staticReport.expected.declarationsSha256,
  },
  definitions: {
    coreFingerprintParity:
      'Exact 42-source registry and source function metadata, plus zero normalized differences in the agent, component, custom-source, hash, confidence, and lazy visitor-id differential across Chrome, Firefox, and Safari.',
    upstreamSideEffectParity:
      'Direct utilities, monitoring, debug logging, iframe rejection/cleanup, preparation, and unstable helper behavior match the unmodified FingerprintJS oracle.',
    observableFunctionMetadata:
      'Public Function.name and Function.length, plus audited returned-function metadata, match the oracle.',
    oracleFormatCoverage:
      'Every JavaScript/declaration artifact present in the oracle dist directory is present; extra formats are allowed.',
  },
  rows,
}

await writeFile(resolve(compareRoot, options.jsonOutput), `${JSON.stringify(report, null, 2)}\n`)
const markdown = renderMarkdown(report)
await writeFile(resolve(compareRoot, options.markdownOutput), markdown)
console.log(markdown)

async function readPhase(id, phase) {
  const results = []
  for (const browser of browsers) {
    const path = resolve(compareRoot, 'results', id, phase, `${browser}.json`)
    if (!existsSync(path)) continue
    results.push(JSON.parse(await readFile(path, 'utf8')))
  }
  return results
}

function annotateDifferences(results) {
  return results.flatMap((result) => {
    const browser = result.browser?.browser ?? result.browser?.name ?? 'unknown'
    return (result.comparison?.differences ?? []).map((difference) => ({ ...difference, browser }))
  })
}

function browserNames(results) {
  return results.map((result) => result.browser?.browser ?? result.browser?.name ?? 'unknown')
}

function isMetadataPath(path) {
  return (
    /\.exportFunctions\.[^.]+\.(?:name|length)$/.test(path) ||
    /\.agent\.get\.(?:name|length)$/.test(path) ||
    /\.unstable\.audio\.(?:name|length)$/.test(path)
  )
}

function summarizeBehavioralIssues(id, differences) {
  const issues = []
  const covered = new Set()
  const monitoring = differences.filter((difference) => difference.path.includes('.monitoring.'))
  if (monitoring.length) {
    monitoring.forEach((difference) => covered.add(difference))
    issues.push({
      code: 'monitoring-no-op',
      severity: 'feature',
      browsers: distinct(monitoring.map((difference) => difference.browser)),
      summary:
        'The oracle sends the sampled npm-monitoring XHR when monitoring is omitted or true; the candidate sends no request. monitoring:false matches.',
    })
  }

  const iframe = differences.filter((difference) => difference.path.includes('.iframe.error'))
  if (iframe.length) {
    iframe.forEach((difference) => covered.add(difference))
    issues.push({
      code: 'with-iframe-sync-throw-does-not-reject',
      severity: 'behavior',
      browsers: distinct(iframe.map((difference) => difference.browser)),
      summary:
        'A synchronously throwing withIframe action rejects with the original TypeError upstream, but the candidate promise did not settle within five seconds.',
    })
  }

  const debug = differences.filter((difference) => difference.path.includes('.debug.'))
  if (debug.length) {
    debug.forEach((difference) => covered.add(difference))
    const isRust = id === 'rust-idiomatic' || id === 'rust-compact' || id === 'rust-hybrid'
    issues.push({
      code: isRust ? 'dom-blockers-debug-log-missing' : id === 'melange' ? 'dom-blockers-debug-mojibake' : 'debug-log-difference',
      severity: 'debug',
      browsers: distinct(debug.map((difference) => difference.browser)),
      summary: isRust
        ? 'With load({debug:true}) on Safari, the oracle logs the DOM-blocker selector report before the agent report; the candidate emits only the agent report.'
        : id === 'melange'
          ? 'The Safari DOM-blocker debug report is emitted, but the arrow/blocked marker glyphs are UTF-8 mojibake rather than the oracle Unicode glyphs.'
          : 'Debug console output differs from the oracle.',
    })
  }

  const remaining = differences.filter((difference) => !covered.has(difference))
  for (const difference of uniqueDifferences(remaining)) {
    issues.push({
      code: 'other-behavioral-difference',
      severity: 'behavior',
      browsers: [difference.browser],
      summary: `${difference.path}: oracle ${shortValue(difference.upstream)}, candidate ${shortValue(difference.candidate)}.`,
    })
  }
  return issues
}

function summarizeMetadataIssues(staticRow, runtimeDifferences) {
  const differences = [
    ...(staticRow.exportMetadataDifferences ?? []).map((difference) => ({
      path: `exports.${difference.key}`,
      upstream: difference.expected,
      candidate: difference.actual,
      browser: 'static ESM',
    })),
    ...runtimeDifferences,
  ]
  const unique = uniqueDifferences(differences).map((difference) => ({
    path: difference.path,
    browsers: [difference.browser],
    upstream: difference.upstream,
    candidate: difference.candidate,
  }))
  const exportedFunctions = distinct(unique.flatMap((difference) => {
    const direct = /^exports\.([^.]+)$/.exec(difference.path)
    if (direct) return [direct[1]]
    const runtime = /\.exportFunctions\.([^.]+)\./.exec(difference.path)
    return runtime ? [runtime[1]] : []
  }))
  const returnedFunctions = distinct(unique.flatMap((difference) => {
    if (difference.path.includes('.agent.get.')) return ['agent.get']
    if (difference.path.includes('.unstable.audio.')) return ['audio cleanup function']
    return []
  }))
  return { differences: unique, exportedFunctions, returnedFunctions }
}

function uniqueDifferences(differences) {
  const seen = new Map()
  for (const difference of differences) {
    const key = JSON.stringify({
      browser: difference.browser,
      path: difference.path,
      upstream: difference.upstream,
      candidate: difference.candidate,
    })
    if (!seen.has(key)) {
      seen.set(key, {
        browser: difference.browser,
        path: difference.path,
        upstream: difference.upstream,
        candidate: difference.candidate,
      })
    }
  }
  return [...seen.values()]
}

function renderMarkdown(report) {
  const lines = [
    '# FingerprintJS feature-parity audit',
    '',
    `Generated ${report.generatedAt}. Oracle commit \`${report.oracleCommit}\`.`,
    '',
    '“Core fingerprinting” is the exact 42-source agent/custom-source/hash contract in all three local browsers. “Upstream effects” additionally includes installation monitoring, debug logs, direct utility calls, iframe error propagation, and cleanup. The latter is intentionally stricter than the historical comparison contract.',
    '',
    '| Implementation | Core fingerprinting | Upstream effects | Function metadata | API keys + `.d.ts` | IIFE shape | Oracle formats | Strict ESM | Artifact package |',
    '| --- | --- | --- | --- | --- | --- | --- | --- | --- |',
  ]
  for (const row of report.rows) {
    lines.push(
      `| ${row.label} | ${mark(row.coreFingerprintParity)} | ${mark(row.upstreamSideEffectParity)} | ${mark(row.observableFunctionMetadata)} | ${mark(row.apiKeysAndTypes)} | ${mark(row.iifeApiShape)} | ${mark(row.oracleFormatCoverage)} | ${mark(row.strictEsmParity)} | ${mark(row.artifactPackageParity)} |`,
    )
  }

  lines.push('', '## Upstream behavioral and side-effect gaps', '')
  for (const row of report.rows.filter((entry) => entry.behavioralIssues.length)) {
    lines.push(`### ${row.label}`, '')
    for (const issue of row.behavioralIssues) {
      lines.push(`- **${issue.code}** (${issue.browsers.join(', ')}): ${issue.summary}`)
    }
    lines.push('')
  }

  lines.push('## Observable function-metadata gaps', '')
  for (const row of report.rows.filter((entry) => entry.metadataIssues.length)) {
    const parts = []
    if (row.metadataSummary.exportedFunctions.length) {
      parts.push(`${row.metadataSummary.exportedFunctions.length} exported function(s)`)
    }
    if (row.metadataSummary.returnedFunctions.length) {
      parts.push(row.metadataSummary.returnedFunctions.join(' and '))
    }
    lines.push(`- **${row.label}:** ${parts.join('; ')} differ in observable name and/or arity. Exact values are in \`feature-parity.json\`.`)
  }

  lines.push('', '## IIFE namespace-shape gaps', '')
  for (const row of report.rows.filter((entry) => !entry.iifeApiShape)) {
    lines.push(`- **${row.label}:** minified IIFE export/default/source property order or default identities differ from the oracle.`)
  }

  lines.push('', '## Missing oracle distribution formats', '')
  for (const row of report.rows.filter((entry) => entry.missingRequiredArtifacts.length)) {
    lines.push(`- **${row.label}:** ${row.missingRequiredArtifacts.map((file) => `\`${file}\``).join(', ')}.`)
  }
  lines.push('')
  return `${lines.join('\n')}\n`
}

function mark(value) {
  return value ? '**PASS**' : 'FAIL'
}

function distinct(values) {
  return [...new Set(values)]
}

function shortValue(value) {
  const text = JSON.stringify(value)
  return text && text.length > 120 ? `${text.slice(0, 117)}...` : text
}

function parseArgs(args) {
  const result = {
    staticInput: 'feature-parity-static.json',
    baselinePhase: 'feature-parity-baseline',
    extendedPhase: 'feature-parity-extended',
    jsonOutput: 'feature-parity.json',
    markdownOutput: 'feature-parity.md',
  }
  for (let index = 0; index < args.length; index += 1) {
    const value = args[index]
    if (value === '--static') result.staticInput = args[++index]
    else if (value === '--baseline-phase') result.baselinePhase = args[++index]
    else if (value === '--extended-phase') result.extendedPhase = args[++index]
    else if (value === '--json') result.jsonOutput = args[++index]
    else if (value === '--markdown') result.markdownOutput = args[++index]
    else throw new Error(`unknown argument: ${value}`)
  }
  return result
}
