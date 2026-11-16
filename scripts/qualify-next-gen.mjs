#!/usr/bin/env node
import {
  cpSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readlinkSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs';
import { execSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { dirname, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { brotliCompressSync, constants as zc, gzipSync } from 'node:zlib';

const WT = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const MAIN_ROOT = '/Users/mikhail.pertsev/Desktop/OSS/mires';
const OUT = resolve(WT, 'build/qualify-next-gen');
const REPO_RECEIPT = resolve(WT, 'experiments/qualification/next-gen-qualify-receipt.json');
const SCRATCH_RECEIPT =
  '/Users/mikhail.pertsev/.gemini/antigravity/brain/5d6afc81-e4f3-4960-a29d-593b8c3ce186/scratch/next-gen-qualify-receipt.json';
const JSC = '/System/Library/Frameworks/JavaScriptCore.framework/Versions/Current/Helpers/jsc';
const BENCH_RS = resolve(MAIN_ROOT, 'build/arch-research/q456/src/bench_kernels.rs');
const BASELINE_PROG = resolve(MAIN_ROOT, 'build/arch-research/q456/baseline/rb_clean_on.mjs');
const BASELINE_RT = resolve(MAIN_ROOT, 'build/arch-research/q456/baseline/rb_clean_on.runtime.mjs');

const AA_RATIO_MIN = 0.88;
const AA_RATIO_MAX = 1.12;
const TIMING_MAX_RETRIES = 3;

function sha256(buf) {
  return createHash('sha256').update(buf).digest('hex');
}

function fileHash(p) {
  const buf = readFileSync(p);
  return { path: p, bytes: buf.length, sha256: sha256(buf) };
}

function readTrim(p) {
  return existsSync(p) ? readFileSync(p, 'utf8').trim() : null;
}

function measureFile(p) {
  const buf = readFileSync(p);
  const text = buf.toString('utf8');
  return {
    bytes: buf.length,
    gzip9: gzipSync(buf, { level: 9 }).length,
    brotli11: brotliCompressSync(buf, { params: { [zc.BROTLI_PARAM_QUALITY]: 11 } }).length,
    tryCount: (text.match(/\btry\s*\{/g) || []).length,
    topLevelFns: (text.match(/^function\s+[A-Za-z0-9_$]+\s*\(/gm) || []).length,
    bigintMentions: (text.match(/\bBigInt\b/g) || []).length,
    sha256: sha256(buf),
  };
}

function runCmd(cmd, args, opts = {}) {
  const t0 = Date.now();
  const res = spawnSync(cmd, args, {
    cwd: opts.cwd || WT,
    env: { ...process.env, ...(opts.env || {}) },
    input: opts.input,
    encoding: 'utf8',
    maxBuffer: 64 * 1024 * 1024,
  });
  return {
    status: res.status,
    elapsedSec: +((Date.now() - t0) / 1000).toFixed(2),
    stdout: res.stdout || '',
    stderr: res.stderr || '',
  };
}

function summarizeLane(lanePath) {
  const lane = JSON.parse(readFileSync(lanePath, 'utf8'));
  return {
    items: lane.items?.length ?? 0,
    fallbackCount: lane.fallback_count ?? 0,
    inlineApplied: lane.inline?.applied ?? 0,
    crossObjectInline: lane.inline?.cross_object_applied ?? 0,
    sroaApplied: lane.passes?.applied?.sroa ?? 0,
    reprApplied: lane.repr?.final_emitted?.applied ?? 0,
    sliceIterWouldLower: lane.slice_iter?.would_lower ?? 0,
    sliceIterEmitted: lane.slice_iter?.final_emitted ?? 0,
    rangeAssertsRemoved: lane.range?.asserts_removed ?? 0,
    rangeComparisonsFolded: lane.range?.comparisons_folded ?? 0,
    witnessesRecorded: lane.witnesses?.recorded ?? 0,
    witnessesFailed: lane.witnesses?.failed ?? 0,
  };
}

const sysrootLinkPath = resolve(WT, 'build/sysroot');
const origSysrootTarget = (() => {
  try {
    return lstatSync(sysrootLinkPath).isSymbolicLink() ? readlinkSync(sysrootLinkPath) : null;
  } catch {
    return null;
  }
})();

const compilerCommit = execSync(`git -C "${WT}" rev-parse HEAD`).toString().trim();
const gitPorcelain = execSync(`git -C "${WT}" status --porcelain`).toString().trim();
const resolvedSysrootDir = origSysrootTarget
  ? resolve(WT, 'build', origSysrootTarget)
  : sysrootLinkPath;

const provenance = {
  compilerCommit,
  gitStatusClean: gitPorcelain === '',
  gitStatusPorcelain: gitPorcelain,
  dylib: fileHash(resolve(WT, 'target/release/librustc_codegen_mires.dylib')),
  jscLink: fileHash(resolve(WT, 'target/release/jsc-link')),
  runtimeShim: fileHash(resolve(WT, 'runtime/shim.js')),
  sysroot: {
    linkTarget: origSysrootTarget,
    cacheKey: readTrim(resolve(resolvedSysrootDir, '.cache-key')),
    target: readTrim(resolve(resolvedSysrootDir, '.target')),
    rustOptLevel: readTrim(resolve(resolvedSysrootDir, '.rust-opt-level')),
    jsArgs: readTrim(resolve(resolvedSysrootDir, '.js-args')),
  },
};

// Wipe all output/target/_build directories completely (never reuse pre-existing artifacts).
rmSync(OUT, { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/fingerprintjs-rust-compact/target'), { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/fingerprintjs-rust-compact/_build'), { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/fingerprintjs-rust/target'), { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/fingerprintjs-rust/_build'), { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/browser-workloads/_build'), { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/browser-workloads/dist'), { recursive: true, force: true });
rmSync(resolve(WT, 'experiments/browser-workloads/results/build.json'), { force: true });
mkdirSync(OUT, { recursive: true });

const fpDistDir = resolve(WT, 'experiments/fingerprintjs/dist');
let createdFpDist = false;
if (
  !existsSync(resolve(fpDistDir, 'fp.esm.js')) &&
  existsSync(resolve(MAIN_ROOT, 'experiments/fingerprintjs/dist/fp.esm.js'))
) {
  cpSync(resolve(MAIN_ROOT, 'experiments/fingerprintjs/dist'), fpDistDir, { recursive: true });
  createdFpDist = true;
}

const nmDir = resolve(WT, 'node_modules');
let createdNmDir = false;
try {
  if (lstatSync(nmDir).isSymbolicLink()) {
    rmSync(nmDir, { force: true });
  }
} catch {}
if (!existsSync(nmDir)) {
  execSync(`cp -cR "${resolve(MAIN_ROOT, 'node_modules')}" "${nmDir}"`);
  createdNmDir = true;
}

function cleanupEnv() {
  if (createdNmDir) {
    try {
      rmSync(nmDir, { recursive: true, force: true });
    } catch {}
  }
  if (createdFpDist) {
    try {
      rmSync(fpDistDir, { recursive: true, force: true });
    } catch {}
  }
  if (origSysrootTarget) {
    try {
      rmSync(sysrootLinkPath, { recursive: true, force: true });
      symlinkSync(origSysrootTarget, sysrootLinkPath);
    } catch {}
  }
}
process.on('exit', cleanupEnv);
process.on('SIGINT', () => {
  cleanupEnv();
  process.exit(130);
});

const results = {
  schema: 1,
  generatedAt: new Date().toISOString(),
  provenance,
  compact: {},
  idiomatic: {},
  corpus: {},
  kernels: {},
  enumStateMatrix: {},
};

function verifyKernelReceipt(kernelOut, label) {
  const k = JSON.parse(readFileSync(kernelOut, 'utf8'));
  const passed = k.checks?.filter((c) => c.passed).length ?? 0;
  const total = k.checks?.length ?? 0;
  if (!k.passed || total !== 58 || passed !== 58) {
    throw new Error(`${label}: expected 58/58 kernels to pass, got ${passed}/${total}`);
  }
  return { passed, total };
}

// 1. FingerprintJS Compact
for (const profile of ['release-balanced', 'release-size']) {
  const distDir = resolve(OUT, `compact-${profile}/dist`);
  const kernelOut = resolve(OUT, `compact-${profile}/kernels.json`);
  console.log(`\n=== [1] Building fingerprintjs-rust-compact (${profile}) ===`);
  const buildRes = runCmd(process.execPath, [
    resolve(WT, 'experiments/fingerprintjs-rust-compact/tools/build.mjs'),
    '--profile',
    profile,
    '--optimizer',
    'on',
    '--lane',
    'jir',
    '--scheduler',
    'boundary',
    '--lane-report',
    '--output',
    distDir,
  ]);
  if (buildRes.status !== 0) {
    throw new Error(`compact (${profile}) build failed:\n${buildRes.stderr}\n${buildRes.stdout}`);
  }
  const checkRes = runCmd(process.execPath, [
    resolve(WT, 'experiments/tools/check-oss-kernels.mjs'),
    resolve(distDir, 'fp.esm.min.js'),
    kernelOut,
  ]);
  if (checkRes.status !== 0) {
    throw new Error(`compact (${profile}) check-oss-kernels failed:\n${checkRes.stderr}\n${checkRes.stdout}`);
  }
  const kernelCounts = verifyKernelReceipt(kernelOut, `compact (${profile})`);
  const min = measureFile(resolve(distDir, 'fp.esm.min.js'));
  const raw = measureFile(resolve(distDir, 'raw/program.js'));
  const lane = summarizeLane(resolve(distDir, 'link-lane-report.json'));
  results.compact[profile] = {
    elapsedSec: buildRes.elapsedSec,
    kernelsExit: checkRes.status,
    kernels: kernelCounts,
    min,
    raw,
    lane,
  };
  console.log(
    `compact (${profile}): ${kernelCounts.passed}/${kernelCounts.total} kernels, min gz9=${min.gzip9} B, br11=${min.brotli11} B, raw fns=${raw.topLevelFns}, try=${raw.tryCount}, inlines=${lane.inlineApplied}, fallbacks=${lane.fallbackCount}, witnessFailed=${lane.witnessesFailed}`,
  );
}

// 2. FingerprintJS Idiomatic
for (const [key, extraArgs] of [
  ['release-balanced-default', []],
  ['release-balanced-oz', ['--rust-opt-level', 'z']],
]) {
  const distDir = resolve(OUT, `idiomatic-${key}/dist`);
  const kernelOut = resolve(OUT, `idiomatic-${key}/kernels.json`);
  console.log(`\n=== [2] Building fingerprintjs-rust idiomatic (${key}) ===`);
  const buildRes = runCmd(process.execPath, [
    resolve(WT, 'experiments/fingerprintjs-rust/tools/build.mjs'),
    '--profile',
    'release-balanced',
    '--optimizer',
    'on',
    '--lane',
    'jir',
    '--scheduler',
    'boundary',
    '--lane-report',
    ...extraArgs,
    '--output',
    distDir,
  ]);
  if (buildRes.status !== 0) {
    throw new Error(`idiomatic (${key}) build failed:\n${buildRes.stderr}\n${buildRes.stdout}`);
  }
  const checkRes = runCmd(process.execPath, [
    resolve(WT, 'experiments/tools/check-oss-kernels.mjs'),
    resolve(distDir, 'boundary/fp.esm.min.js'),
    kernelOut,
  ]);
  if (checkRes.status !== 0) {
    throw new Error(`idiomatic (${key}) check-oss-kernels failed:\n${checkRes.stderr}\n${checkRes.stdout}`);
  }
  const kernelCounts = verifyKernelReceipt(kernelOut, `idiomatic (${key})`);
  const min = measureFile(resolve(distDir, 'boundary/fp.esm.min.js'));
  const raw = measureFile(resolve(distDir, 'boundary/raw/program.js'));
  const lane = summarizeLane(resolve(distDir, 'boundary/link-lane-report.json'));
  results.idiomatic[key] = {
    elapsedSec: buildRes.elapsedSec,
    kernelsExit: checkRes.status,
    kernels: kernelCounts,
    min,
    raw,
    lane,
  };
  console.log(
    `idiomatic (${key}): ${kernelCounts.passed}/${kernelCounts.total} kernels, min gz9=${min.gzip9} B, br11=${min.brotli11} B, raw fns=${raw.topLevelFns}, try=${raw.tryCount}, inlines=${lane.inlineApplied}, fallbacks=${lane.fallbackCount}, witnessFailed=${lane.witnessesFailed}`,
  );
}

// 3. Browser Workloads Corpus (all 10 workloads + isolated A/A-gated Node timing retries)
console.log('\n=== [3a] Building all 10 browser-workloads ===');
const corpusBuild = runCmd(process.execPath, [
  resolve(WT, 'experiments/browser-workloads/tools/build.mjs'),
]);
if (corpusBuild.status !== 0) {
  throw new Error(`browser-workloads build failed:\n${corpusBuild.stderr}\n${corpusBuild.stdout}`);
}

const corpusRuntimeJson = resolve(OUT, 'corpus-runtime.json');
console.log('\n=== [3b] Running browser-workloads --pure ===');
const corpusRun = runCmd(process.execPath, [
  resolve(WT, 'experiments/browser-workloads/tools/run.mjs'),
  '--pure',
  '--samples',
  '9',
  '--warmups',
  '2',
  '--output',
  corpusRuntimeJson,
]);
if (corpusRun.status !== 0) {
  throw new Error(`browser-workloads run.mjs --pure failed:\n${corpusRun.stderr}\n${corpusRun.stdout}`);
}
console.log(corpusRun.stdout.trim());

const runtimeReceipt = JSON.parse(readFileSync(corpusRuntimeJson, 'utf8'));
const timingNode = runtimeReceipt.timing?.environments?.node ?? runtimeReceipt.timing;

// Build timing inputs matching experiments/browser-workloads/tools/run.mjs
const { cellSeed } = await import(
  pathToFileURL(resolve(WT, 'experiments/browser-workloads/tools/timing-protocol.mjs')).href
);
function makeDocuments(count) {
  return Array.from(
    { length: count },
    (_, index) =>
      `d${index}|Rust browser compiler ${index}|Offline browser tools and compiler experiments`,
  ).join('\n');
}
function makeMarkdown(count) {
  return Array.from(
    { length: count },
    (_, index) =>
      `## Section ${index}\n\n- **browser** compiler\n- [report](https://example.test/${index})\n\nText with <escape>.`,
  ).join('\n\n');
}
const markdown80 = makeMarkdown(80);
const timedInputs = {
  marked: { markdown: markdown80 },
  minisearch: { documents: makeDocuments(12), query: 'browser comp*' },
  fflate: { text: markdown80.repeat(8) },
};

const timingCellScript = resolve(WT, 'experiments/browser-workloads/tools/timing-cell.mjs');
const timingSummary = {};
const timingRetries = {};

function runIsolatedCell(workload, pairSpec, seedNum, expectedChecksum, expectedInputSha256) {
  const arms = pairSpec.arms.map((arm) => ({
    key: arm.key,
    label: arm.label,
    implementation: arm.implementation,
    url:
      arm.implementation === 'rust'
        ? `${pathToFileURL(resolve(WT, `experiments/browser-workloads/dist/${workload}/index.min.js`)).href}?instance=${arm.instance}`
        : pathToFileURL(resolve(WT, `experiments/browser-workloads/reference/${workload}.mjs`)).href,
  }));
  const cfg = {
    workload,
    pair: pairSpec.pair,
    arms,
    input: timedInputs[workload],
    expectedInputSha256,
    expectedChecksum,
    warmups: 2,
    samples: 9,
    minSampleMs: 20,
    seed: seedNum,
  };
  const res = runCmd(process.execPath, [timingCellScript], { input: JSON.stringify(cfg) });
  if (res.status !== 0) {
    throw new Error(`timing-cell.mjs failed for ${workload} ${pairSpec.pair}: ${res.stderr}`);
  }
  return JSON.parse(res.stdout);
}

const PAIR_DEFS = [
  {
    pair: 'rust-vs-js',
    arms: [
      { key: 'rust', label: 'R', implementation: 'rust', instance: 'a' },
      { key: 'js', label: 'J', implementation: 'js' },
    ],
  },
  {
    pair: 'rust-vs-rust',
    arms: [
      { key: 'rust-a', label: 'A', implementation: 'rust', instance: 'a' },
      { key: 'rust-b', label: 'B', implementation: 'rust', instance: 'b' },
    ],
  },
];

for (const workload of ['marked', 'minisearch', 'fflate']) {
  const cells = timingNode.cells || [];
  const rjIdx = cells.findIndex((c) => c.workload === workload && c.pair === 'rust-vs-js');
  const aaIdx = cells.findIndex((c) => c.workload === workload && c.pair === 'rust-vs-rust');
  if (rjIdx < 0 || aaIdx < 0) continue;

  const initialAa = cells[aaIdx].ratio?.p50 ?? null;
  const initialRj = cells[rjIdx].ratio?.p50 ?? null;
  const expectedChecksum = cells[rjIdx].checksum;
  const expectedInputSha256 = runtimeReceipt.timing?.workloads?.[workload]?.input?.sha256 ?? null;
  const attempts = [];

  let chosenRj = cells[rjIdx];
  let chosenAa = cells[aaIdx];
  const inBand = (r) => typeof r === 'number' && r >= AA_RATIO_MIN && r <= AA_RATIO_MAX;

  if (!inBand(initialAa)) {
    console.log(
      `timing retry triggered for ${workload}: initial A/A p50 ratio=${initialAa} outside [${AA_RATIO_MIN}, ${AA_RATIO_MAX}]`,
    );
    let bestDelta = Math.abs(1 - (initialAa ?? 99));
    for (let attempt = 1; attempt <= TIMING_MAX_RETRIES; attempt++) {
      // Brief pause between isolated attempts to let background CPU settle
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1000);
      const rjCell = runIsolatedCell(
        workload,
        PAIR_DEFS[0],
        cellSeed(1 + attempt * 100, 'node', workload, 'rust-vs-js'),
        expectedChecksum,
        expectedInputSha256,
      );
      const aaCell = runIsolatedCell(
        workload,
        PAIR_DEFS[1],
        cellSeed(1 + attempt * 100, 'node', workload, 'rust-vs-rust'),
        expectedChecksum,
        expectedInputSha256,
      );
      const aaP50 = aaCell.ratio?.p50 ?? null;
      const rjP50 = rjCell.ratio?.p50 ?? null;
      attempts.push({ attempt, aaRatioP50: aaP50, rustJsRatioP50: rjP50 });
      console.log(`  attempt ${attempt}: ${workload} A/A=${aaP50}, rust/js=${rjP50}`);
      if (rjCell.status === 'ok' && aaCell.status === 'ok') {
        const delta = Math.abs(1 - (aaP50 ?? 99));
        if (inBand(aaP50) || delta < bestDelta) {
          chosenRj = rjCell;
          chosenAa = aaCell;
          bestDelta = delta;
        }
        if (inBand(aaP50)) break;
      }
    }
    cells[rjIdx] = chosenRj;
    cells[aaIdx] = chosenAa;
  }

  timingRetries[workload] = attempts;
  timingSummary[workload] = {
    initialAaRatio: initialAa,
    initialRustJsRatio: initialRj,
    finalAaRatio: chosenAa.ratio?.p50 ?? null,
    finalRustJsRatio: chosenRj.ratio?.p50 ?? null,
    aaWithinGate: inBand(chosenAa.ratio?.p50),
    retryCount: attempts.length,
  };
}

const corpusWorkloads = {};
for (const name of [
  'marked',
  'minisearch',
  'fflate',
  'uplot',
  'idb',
  'idb_lines',
  'ky',
  'explorer',
  'workbox',
  'workbox_direct',
]) {
  const min = measureFile(resolve(WT, `experiments/browser-workloads/dist/${name}/index.min.js`));
  const raw = measureFile(resolve(WT, `experiments/browser-workloads/dist/${name}/raw/program.js`));
  const lanePath = resolve(WT, `experiments/browser-workloads/_build/${name}/program.js.lane.json`);
  const lane = existsSync(lanePath) ? summarizeLane(lanePath) : null;
  corpusWorkloads[name] = { min, raw, lane };
}
results.corpus = {
  buildElapsedSec: corpusBuild.elapsedSec,
  runElapsedSec: corpusRun.elapsedSec,
  status: runtimeReceipt.status,
  workloads: corpusWorkloads,
  timingNode,
  timingSummary,
  timingRetries,
};
for (const name of ['marked', 'minisearch', 'fflate']) {
  const w = corpusWorkloads[name];
  const ts = timingSummary[name];
  console.log(
    `corpus ${name}: min gz9=${w.min.gzip9} B, br11=${w.min.brotli11} B, raw fns=${w.raw.topLevelFns}, try=${w.raw.tryCount}, bigints=${w.raw.bigintMentions}, sliceIter=${w.lane?.sliceIterEmitted ?? '?'}, rangeRemoved=${w.lane?.rangeAssertsRemoved ?? '?'}, rust/js=${ts.finalRustJsRatio?.toFixed(3)}, A/A=${ts.finalAaRatio?.toFixed(3)} (gate=${ts.aaWithinGate})`,
  );
}

// Shared standalone harness helpers for Sections 4 & 5
const stripModuleSyntax = (s) =>
  s
    .replace(/^import\s*\*\s*as\s+__rt\s+from[^\n]+$/gm, '')
    .replace(/\bexport\s*\{[^}]+\};?/g, '')
    .replace(/\bexport\s+function\s+/g, 'function ')
    .replace(/\bexport\s+(?:var|const|let)\s+/g, 'var ');

function makeStandaloneHarness(shimSrc, progSrc, fnName, callArgs, unsigned = true, extraPreamble = '') {
  const norm = unsigned ? '>>> 0' : '| 0';
  return `${extraPreamble}
${stripModuleSyntax(shimSrc)}
const __rt = globalThis.__rt;
${stripModuleSyntax(progSrc)}
;{
  const __f = ${fnName};
  const __args = ${JSON.stringify(callArgs)};
  const __now = typeof preciseTime === "function" ? () => preciseTime() * 1000 : () => performance.now();
  const __out = typeof print === "function" ? print : (s) => console.log(s);
  const __v = (__f(...__args)) ${norm};
  let __sink = 0, __n = 0;
  const __w0 = __now();
  while (__n < 250 || __now() - __w0 < 200) {
    __sink ^= (__f(...__args)) | 0;
    __n++;
  }
  const __avgMs = (__now() - __w0) / __n;
  const __per = Math.max(1, Math.min(20000, Math.round(15 / Math.max(__avgMs, 0.0005))));
  const __samples = [];
  for (let r = 0; r < 19; r++) {
    const t0 = __now();
    for (let i = 0; i < __per; i++) __sink ^= (__f(...__args)) | 0;
    __samples.push(((__now() - t0) * 1000) / __per);
  }
  __samples.sort((a, b) => a - b);
  __out(JSON.stringify({
    ok: true,
    value: __v,
    perRound: __per,
    medianUs: +__samples[9].toFixed(3),
    p10Us: +__samples[1].toFixed(3),
    minUs: +__samples[0].toFixed(3),
    maxUs: +__samples[18].toFixed(3),
    sink: __sink,
  }));
}
`;
}

// 4. Microkernels (bench_kernels.rs)
console.log('\n=== [4] Compiling & timing bench_kernels.rs in Node V8 and standalone JSC ===');
const kernelDir = resolve(OUT, 'kernels');
mkdirSync(kernelDir, { recursive: true });
const kernelJs = resolve(kernelDir, 'bench_kernels.js');
const sysrootEnv = {
  JSC_RUST_OPT_LEVEL: 'z',
  JSC_SYSROOT_ARGS: 'js-profile=release-balanced js-opt=on js-ecma-target=2018 js-cleanup=off',
  JS_EXTRA_ARGS: 'js-lane=jir js-profile=release-balanced js-opt=on js-ecma-target=2018 js-cleanup=off js-modules=esm',
  SKIP_BUILD: '1',
};
const ensureK = runCmd('bash', [resolve(WT, 'scripts/build_sysroot.sh')], { env: sysrootEnv });
if (ensureK.status !== 0) throw new Error(`kernel build_sysroot failed:\n${ensureK.stderr}`);
const compK = runCmd(
  'bash',
  [resolve(WT, 'scripts/compile_core.sh'), BENCH_RS, kernelJs, '--crate-type', 'cdylib'],
  { env: sysrootEnv },
);
if (compK.status !== 0) throw new Error(`kernel compile_core failed:\n${compK.stderr}`);

const minShimPath = resolve(kernelDir, 'shim.min.js');
const shimGen = runCmd(process.execPath, [
  resolve(WT, 'scripts/minimal-shim.mjs'),
  resolve(WT, 'runtime/shim.js'),
  minShimPath,
  kernelJs,
]);
if (shimGen.status !== 0) throw new Error(`minimal-shim failed:\n${shimGen.stderr}`);

const minShimEsmPath = resolve(kernelDir, 'shim.min.mjs');
const shimEsmGen = runCmd(process.execPath, [
  resolve(WT, 'scripts/minimal-shim.mjs'),
  '--module',
  resolve(WT, 'runtime/shim.js'),
  minShimEsmPath,
  kernelJs,
]);
if (shimEsmGen.status !== 0) throw new Error(`minimal-shim --module failed:\n${shimEsmGen.stderr}`);

const rawKernelCode = readFileSync(kernelJs, 'utf8');
const minShimCode = readFileSync(minShimPath, 'utf8');
const fullShimCode = readFileSync(resolve(WT, 'runtime/shim.js'), 'utf8');

const { minify } = await import(pathToFileURL(resolve(MAIN_ROOT, 'node_modules/terser/main.js')).href);
const combinedEsm = `${stripModuleSyntax(minShimCode)}\nconst __rt = globalThis.__rt;\n${rawKernelCode.replace(/^import\s*\*\s*as\s+__rt\s+from[^\n]+$/m, '')}`;
const minKernelRes = await minify(combinedEsm, {
  ecma: 2018,
  module: true,
  safari10: true,
  format: { comments: false },
});
const minKernelPath = resolve(kernelDir, 'bench_kernels.bundle.min.js');
writeFileSync(minKernelPath, minKernelRes.code + '\n');

results.kernels.artifacts = {
  rawProgram: measureFile(kernelJs),
  minimalRuntime: measureFile(minShimPath),
  minimalRuntimeEsm: measureFile(minShimEsmPath),
  minifiedBundleWithMinimalShim: measureFile(minKernelPath),
  zeroBigIntInProgram: (rawKernelCode.match(/\bBigInt\b/g) || []).length === 0,
  crcTableHoistedAtTopLevel: /^(?:const|let|var)\s+\$[A-Za-z0-9_$]*\s*=\s*\[0,\s*1996959894,/m.test(rawKernelCode),
};
console.log(
  `kernel bundle: min gz9=${results.kernels.artifacts.minifiedBundleWithMinimalShim.gzip9} B, zeroBigInt=${results.kernels.artifacts.zeroBigIntInProgram}`,
);

const jsRefMod = await import(pathToFileURL(resolve(MAIN_ROOT, 'build/arch-research/q456/refs/js_ref.mjs')).href);
const baselineProgCode = existsSync(BASELINE_PROG) ? readFileSync(BASELINE_PROG, 'utf8') : null;
const baselineRtCode = existsSync(BASELINE_RT) ? readFileSync(BASELINE_RT, 'utf8') : null;
const jscAsciiPolyfillForLegacyMainOnly = `
if (typeof TextEncoder === "undefined") {
  globalThis.TextEncoder = class { encode(s) { const u = new Uint8Array(s.length); for (let i = 0; i < s.length; i++) u[i] = s.charCodeAt(i) & 0xff; return u; } };
  globalThis.TextDecoder = class { decode(u) { let s = ""; for (let i = 0; i < u.length; i++) s += String.fromCharCode(u[i]); return s; } };
}
`;

for (const [kName, fnName, callArgs, unsigned] of [
  ['murmur_u64_4096', 'bench_murmur_u64', [4096, 42], true],
  ['crc32_table_8192', 'bench_crc32_table', [8192], true],
  ['crc32_table_16384', 'bench_crc32_table', [16384], true],
  ['enum_state_4096', 'bench_enum_state', [4096], false],
  ['string_vec_builder_200', 'bench_string_vec_builder', [200], true],
]) {
  const expectedVal = unsigned ? jsRefMod[fnName](...callArgs) >>> 0 : jsRefMod[fnName](...callArgs) | 0;
  const optHarness = resolve(kernelDir, `${kName}.opt.harness.js`);
  writeFileSync(optHarness, makeStandaloneHarness(fullShimCode, rawKernelCode, fnName, callArgs, unsigned, ''));

  let baseV8Obj = null;
  let baseJscObj = null;
  if (baselineProgCode && baselineRtCode) {
    const baseHarnessV8 = resolve(kernelDir, `${kName}.base.v8.js`);
    writeFileSync(baseHarnessV8, makeStandaloneHarness(baselineRtCode, baselineProgCode, fnName, callArgs, unsigned, ''));
    const baseHarnessJsc = resolve(kernelDir, `${kName}.base.jsc.js`);
    writeFileSync(
      baseHarnessJsc,
      makeStandaloneHarness(
        baselineRtCode,
        baselineProgCode,
        fnName,
        callArgs,
        unsigned,
        jscAsciiPolyfillForLegacyMainOnly,
      ),
    );

    const bV8 = runCmd(process.execPath, [baseHarnessV8]);
    if (bV8.status === 0) baseV8Obj = JSON.parse(bV8.stdout.trim().split('\n').pop());
    const bJsc = runCmd(JSC, [baseHarnessJsc]);
    if (bJsc.status === 0) baseJscObj = JSON.parse(bJsc.stdout.trim().split('\n').pop());
  }

  const nodeRun = runCmd(process.execPath, [optHarness]);
  if (nodeRun.status !== 0) throw new Error(`Opt Node failed on ${kName}:\n${nodeRun.stderr}`);
  const nodeObj = JSON.parse(nodeRun.stdout.trim().split('\n').pop());

  const jscRun = runCmd(JSC, [optHarness]);
  if (jscRun.status !== 0) throw new Error(`Opt JSC failed on ${kName}:\n${jscRun.stderr}\n${jscRun.stdout}`);
  const jscObj = JSON.parse(jscRun.stdout.trim().split('\n').pop());

  if (nodeObj.value !== jscObj.value || nodeObj.value !== expectedVal) {
    throw new Error(
      `Value mismatch on ${kName}: optV8=${nodeObj.value}, optJsc=${jscObj.value}, expected=${expectedVal}`,
    );
  }

  results.kernels[kName] = {
    value: nodeObj.value,
    v8Baseline: baseV8Obj,
    v8Opt: nodeObj,
    v8SpeedupMedian: baseV8Obj ? +(baseV8Obj.medianUs / nodeObj.medianUs).toFixed(2) : null,
    jscBaseline: baseJscObj,
    jscOpt: jscObj,
    jscSpeedupMedian: baseJscObj ? +(baseJscObj.medianUs / jscObj.medianUs).toFixed(2) : null,
  };
  console.log(
    `kernel ${kName} (val=${nodeObj.value}): V8 ${baseV8Obj?.medianUs ?? '?'} -> ${nodeObj.medianUs} us (${results.kernels[kName].v8SpeedupMedian}x) | JSC ${baseJscObj?.medianUs ?? '?'} -> ${jscObj.medianUs} us (${results.kernels[kName].jscSpeedupMedian}x)`,
  );
}

// 5. Explicit 2x2 enum_state_4096 Matrix (Old main compiler vs New feat/next-gen-qualify x {opaque_helper, local_inlinable_helper})
console.log('\n=== [5] Running 2x2 enum_state_4096 Matrix (old main vs new qualify x opaque/local) ===');
const matrixDir = resolve(OUT, 'enum-matrix');
mkdirSync(matrixDir, { recursive: true });

const makeEnumSource = (inlineNever) => `#![no_std]
#![no_main]

enum State {
    Idle,
    Loading { progress: i32, retries: i32 },
    Ready { value: i32, flags: i32, checksum: i32 },
}

${inlineNever ? '#[inline(never)]\n' : ''}fn step_state(state: &mut State, i: i32, acc: &mut i32) {
    match *state {
        State::Idle => {
            *state = State::Loading { progress: i, retries: 1 };
        }
        State::Loading { progress, retries } => {
            if (i & 3) == 3 {
                *state = State::Ready {
                    value: progress.wrapping_add(i),
                    flags: retries,
                    checksum: i.wrapping_mul(31),
                };
            } else {
                *state = State::Loading {
                    progress: progress.wrapping_add(i),
                    retries: retries.wrapping_add(1),
                };
            }
        }
        State::Ready { value, flags, checksum } => {
            *acc = acc.wrapping_add(value).wrapping_add(flags).wrapping_add(checksum);
            *state = State::Idle;
        }
    }
}

#[unsafe(no_mangle)]
pub fn bench_enum_state(n: i32) -> i32 {
    let mut state = State::Idle;
    let mut acc = 0;
    for i in 0..n {
        step_state(&mut state, i, &mut acc);
    }
    acc
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {}
}
`;

const enumSources = {
  opaque_helper: resolve(matrixDir, 'enum_opaque.rs'),
  local_inlinable_helper: resolve(matrixDir, 'enum_local.rs'),
};
writeFileSync(enumSources.opaque_helper, makeEnumSource(true));
writeFileSync(enumSources.local_inlinable_helper, makeEnumSource(false));

const MAIN_DYLIB = resolve(MAIN_ROOT, 'target/release/librustc_codegen_mires.dylib');
const MAIN_JSC_LINK = resolve(MAIN_ROOT, 'target/release/jsc-link');
const MAIN_RB_SYSROOT = resolve(
  MAIN_ROOT,
  'build/sysroots/b845d7a91c66afe024a793f544e1643a22e071f479569f53c30b21d43d2d5d15',
);
const MAIN_DEFAULT_SYSROOT = resolve(MAIN_ROOT, 'build/sysroot');
const mainSysroot = existsSync(MAIN_RB_SYSROOT) ? MAIN_RB_SYSROOT : MAIN_DEFAULT_SYSROOT;
const mainSysrootArgs = readTrim(resolve(mainSysroot, '.js-args')) || '';
const mainShimCode = readFileSync(resolve(MAIN_ROOT, 'runtime/shim.js'), 'utf8');

for (const [variant, srcPath] of Object.entries(enumSources)) {
  results.enumStateMatrix[variant] = {};

  // 5a. Old main compiler (STRICTLY READ-ONLY against MAIN_ROOT, compiling into matrixDir)
  const oldOut = resolve(matrixDir, `old_main.${variant}.js`);
  const oldWork = resolve(matrixDir, `old_main_${variant}_work`);
  mkdirSync(oldWork, { recursive: true });
  const oldArgs = [
    '+nightly-2026-07-27',
    `-Zcodegen-backend=${MAIN_DYLIB}`,
    '--edition',
    '2021',
    '--target',
    'wasm32-unknown-unknown',
    '--sysroot',
    mainSysroot,
    '--crate-type',
    'cdylib',
    '-Ccodegen-units=1',
    '-Cpanic=abort',
    '-Coverflow-checks=off',
    '-Cdebug-assertions=off',
    '-Cdebuginfo=0',
    ...(readTrim(resolve(mainSysroot, '.rust-opt-level'))
      ? [`-Copt-level=${readTrim(resolve(mainSysroot, '.rust-opt-level'))}`]
      : []),
    `-Cllvm-args=js-lane=jir ${mainSysrootArgs} js-modules=esm`.trim(),
    '-o',
    oldOut,
    srcPath,
  ];
  const oldComp = runCmd('rustc', oldArgs, {
    cwd: oldWork,
    env: { JSC_LINK_BIN: MAIN_JSC_LINK },
  });
  if (oldComp.status !== 0) {
    throw new Error(`old_main compile failed on ${variant}:\n${oldComp.stderr}\n${oldComp.stdout}`);
  }
  const oldProgCode = readFileSync(oldOut, 'utf8');
  const oldHarnessV8 = resolve(matrixDir, `old_main.${variant}.v8.js`);
  const oldHarnessJsc = resolve(matrixDir, `old_main.${variant}.jsc.js`);
  writeFileSync(oldHarnessV8, makeStandaloneHarness(mainShimCode, oldProgCode, 'bench_enum_state', [4096], false, ''));
  writeFileSync(
    oldHarnessJsc,
    makeStandaloneHarness(
      mainShimCode,
      oldProgCode,
      'bench_enum_state',
      [4096],
      false,
      jscAsciiPolyfillForLegacyMainOnly,
    ),
  );
  const oldV8 = JSON.parse(runCmd(process.execPath, [oldHarnessV8]).stdout.trim().split('\n').pop());
  const oldJsc = JSON.parse(runCmd(JSC, [oldHarnessJsc]).stdout.trim().split('\n').pop());
  if (oldV8.value !== 71197732 || oldJsc.value !== 71197732) {
    throw new Error(`old_main ${variant} checksum mismatch: v8=${oldV8.value}, jsc=${oldJsc.value}`);
  }
  results.enumStateMatrix[variant].old_main = {
    sysrootUsed: mainSysroot,
    llvmArgs: `js-lane=jir ${mainSysrootArgs} js-modules=esm`.trim(),
    artifact: measureFile(oldOut),
    v8: oldV8,
    jsc: oldJsc,
  };

  // 5b. New feat/next-gen-qualify compiler
  const newOut = resolve(matrixDir, `new_qualify.${variant}.js`);
  const newComp = runCmd(
    'bash',
    [resolve(WT, 'scripts/compile_core.sh'), srcPath, newOut, '--crate-type', 'cdylib'],
    { env: sysrootEnv },
  );
  if (newComp.status !== 0) {
    throw new Error(`new_qualify compile failed on ${variant}:\n${newComp.stderr}\n${newComp.stdout}`);
  }
  const newProgCode = readFileSync(newOut, 'utf8');
  const newHarness = resolve(matrixDir, `new_qualify.${variant}.harness.js`);
  writeFileSync(newHarness, makeStandaloneHarness(fullShimCode, newProgCode, 'bench_enum_state', [4096], false, ''));
  const newV8 = JSON.parse(runCmd(process.execPath, [newHarness]).stdout.trim().split('\n').pop());
  const newJsc = JSON.parse(runCmd(JSC, [newHarness]).stdout.trim().split('\n').pop());
  if (newV8.value !== 71197732 || newJsc.value !== 71197732) {
    throw new Error(`new_qualify ${variant} checksum mismatch: v8=${newV8.value}, jsc=${newJsc.value}`);
  }
  results.enumStateMatrix[variant].new_qualify = {
    llvmArgs: sysrootEnv.JS_EXTRA_ARGS,
    artifact: measureFile(newOut),
    v8: newV8,
    jsc: newJsc,
  };

  results.enumStateMatrix[variant].speedup = {
    v8Median: +(oldV8.medianUs / newV8.medianUs).toFixed(2),
    jscMedian: +(oldJsc.medianUs / newJsc.medianUs).toFixed(2),
  };

  console.log(
    `2x2 enum_state_4096 [${variant}]: V8 ${oldV8.medianUs} -> ${newV8.medianUs} us (${results.enumStateMatrix[variant].speedup.v8Median}x) | JSC ${oldJsc.medianUs} -> ${newJsc.medianUs} us (${results.enumStateMatrix[variant].speedup.jscMedian}x) [val=71197732]`,
  );
}

mkdirSync(dirname(REPO_RECEIPT), { recursive: true });
mkdirSync(dirname(SCRATCH_RECEIPT), { recursive: true });
writeFileSync(REPO_RECEIPT, JSON.stringify(results, null, 2) + '\n');
writeFileSync(SCRATCH_RECEIPT, JSON.stringify(results, null, 2) + '\n');
cleanupEnv();
console.log(`\nSaved qualification receipt to:\n  - ${REPO_RECEIPT}\n  - ${SCRATCH_RECEIPT}`);
