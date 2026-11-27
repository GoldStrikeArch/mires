#!/usr/bin/env node
// Continuous Differential Oracle runner for Mires (Rust -> JS).
//
// Runs `cargo mires test --diff` across:
//   1. `rustc-ui`   — curated rustc/tests/ui-style run-pass suite
//   2. `coretests`  — real upstream library/coretests modules from rust-src
//   3. `alloctests` — real upstream library/alloctests modules from rust-src
//   4. `crates`     — 16 popular pure-Rust crates.io suites (pinned Cargo.lock)
//
// Writes:
//   - build/oracle/receipt.json
//   - build/oracle/scorecard.md
//
// Fails non-zero on any compile failure, runtime failure, or native-vs-Mires mismatch.

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const REPO_ROOT = path.resolve(__dirname, "..");

const ALL_SUITES = ["rustc-ui", "coretests", "alloctests", "crates"];
const FAST_CRATES = [
  "serde_json",
  "base64",
  "miniz_oxide",
  "csv",
  "itertools",
  "pulldown_cmark",
];
const ALL_CRATES = [
  "adler2",
  "base64",
  "bitflags",
  "crc32fast",
  "csv",
  "flate2",
  "fnv",
  "hex",
  "itertools",
  "miniz_oxide",
  "ordered_float",
  "percent_encoding",
  "pulldown_cmark",
  "serde_json",
  "smallvec",
  "unicode_ident",
];

function parseArgs(argv) {
  let tier = "fast";
  let only = null;
  let profiles = null;
  let outDir = path.join(REPO_ROOT, "build", "oracle");
  let cargoMires = path.join(
    REPO_ROOT,
    "target",
    "release",
    process.platform === "win32" ? "cargo-mires.exe" : "cargo-mires",
  );
  let verbose = false;

  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--tier" && i + 1 < argv.length) {
      tier = argv[++i];
    } else if (a.startsWith("--tier=")) {
      tier = a.slice("--tier=".length);
    } else if (a === "--only" && i + 1 < argv.length) {
      only = argv[++i].split(",").map((s) => s.trim()).filter(Boolean);
    } else if (a.startsWith("--only=")) {
      only = a.slice("--only=".length).split(",").map((s) => s.trim()).filter(Boolean);
    } else if (a === "--profiles" && i + 1 < argv.length) {
      profiles = argv[++i].split(",").map((s) => s.trim()).filter(Boolean);
    } else if (a.startsWith("--profiles=")) {
      profiles = a.slice("--profiles=".length).split(",").map((s) => s.trim()).filter(Boolean);
    } else if (a === "--out-dir" && i + 1 < argv.length) {
      outDir = path.resolve(argv[++i]);
    } else if (a === "--cargo-mires" && i + 1 < argv.length) {
      cargoMires = path.resolve(argv[++i]);
    } else if (a === "--verbose" || a === "-v") {
      verbose = true;
    } else if (a === "--help" || a === "-h") {
      console.log(`Usage: node oracle/run-oracle.mjs [OPTIONS]

Options:
  --tier <fast|full>         Oracle tier (default: fast)
  --only <s1,s2,...>         Subset of suites: ${ALL_SUITES.join(",")}
  --profiles <p1,p2>         Profiles: dev, release-balanced
  --out-dir <path>           Output directory (default: build/oracle)
  --cargo-mires <path>       Path to cargo-mires binary
  -v, --verbose              Stream full cargo-mires output
`);
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${a}`);
    }
  }

  if (tier !== "fast" && tier !== "full") {
    throw new Error(`Invalid --tier '${tier}'; expected 'fast' or 'full'`);
  }
  const selectedSuites = only ?? ALL_SUITES;
  for (const s of selectedSuites) {
    if (!ALL_SUITES.includes(s)) {
      throw new Error(`Unknown suite '${s}'; valid suites: ${ALL_SUITES.join(", ")}`);
    }
  }
  const selectedProfiles = profiles ?? (tier === "full" ? ["dev", "release-balanced"] : ["dev"]);
  for (const p of selectedProfiles) {
    if (p !== "dev" && p !== "release-balanced") {
      throw new Error(`Unsupported profile '${p}'; expected 'dev' or 'release-balanced'`);
    }
  }

  return {
    tier,
    suites: selectedSuites,
    profiles: selectedProfiles,
    outDir,
    cargoMires,
    verbose,
  };
}

function getRustcSysroot() {
  const res = spawnSync("rustc", ["--print", "sysroot"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
  });
  if (res.status !== 0) {
    throw new Error(`rustc --print sysroot failed: ${res.stderr}`);
  }
  return res.stdout.trim();
}

function getRustcVersion() {
  const res = spawnSync("rustc", ["--version"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
  });
  return res.status === 0 ? res.stdout.trim() : "unknown";
}

function getGitSha() {
  const res = spawnSync("git", ["rev-parse", "--short", "HEAD"], {
    cwd: REPO_ROOT,
    encoding: "utf8",
  });
  return res.status === 0 ? res.stdout.trim() : "unknown";
}

function symlinkUpstreamEntries(srcDir, dstSrcDir) {
  fs.mkdirSync(dstSrcDir, { recursive: true });
  for (const entry of fs.readdirSync(srcDir)) {
    if (entry === "lib.rs") continue;
    const dst = path.join(dstSrcDir, entry);
    if (fs.existsSync(dst) || fs.lstatSync(dst, { throwIfNoEntry: false })) {
      fs.rmSync(dst, { recursive: true, force: true });
    }
    fs.symlinkSync(path.join(srcDir, entry), dst);
  }
}

function generateCoretestsHarness(workDir, sysroot, tier, profile) {
  const skiplist = JSON.parse(
    fs.readFileSync(path.join(__dirname, "coretests", "skiplist.json"), "utf8"),
  );
  const upstreamTestsDir = path.join(
    sysroot,
    "lib",
    "rustlib",
    "src",
    "rust",
    "library",
    "coretests",
    "tests",
  );
  if (!fs.existsSync(upstreamTestsDir)) {
    throw new Error(
      `Upstream rust-src coretests not found at ${upstreamTestsDir}. Install component 'rust-src'.`,
    );
  }

  const isRelease = profile === "release-balanced";
  const pkgDir = path.join(workDir, `coretests-${tier}-${profile}`);
  fs.rmSync(pkgDir, { recursive: true, force: true });
  fs.mkdirSync(path.join(pkgDir, "src"), { recursive: true });
  fs.writeFileSync(
    path.join(pkgDir, "Cargo.toml"),
    `[workspace]\n[package]\nname = "mires_oracle_coretests"\nversion = "0.1.0"\nedition = "${skiplist.edition}"\n`,
  );
  fs.writeFileSync(path.join(pkgDir, "src", "lib.rs"), "");

  const upstreamLib = fs.readFileSync(path.join(upstreamTestsDir, "lib.rs"), "utf8");
  const stripped = new Set(skiplist.stripped_features.map((f) => f.feature));
  const headerLines = ["#![allow(warnings)]"];
  for (const line of upstreamLib.split("\n")) {
    if (line.startsWith("mod ")) break;
    let skipLine = false;
    for (const feat of stripped) {
      if (line.includes(`feature(${feat})`)) {
        skipLine = true;
        break;
      }
    }
    if (!skipLine) headerLines.push(line);
  }
  const headerText = headerLines.join("\n") + "\n\n";

  function writeShard(shardName, body) {
    const shardDir = path.join(pkgDir, "tests", shardName);
    symlinkUpstreamEntries(upstreamTestsDir, shardDir);
    fs.writeFileSync(path.join(shardDir, "main.rs"), headerText + body + "\n");
  }

  const releaseSkipped = new Set(Object.keys(skiplist.release_skipped_modules ?? {}));
  const rawTopMods =
    tier === "fast" ? skiplist.fast_tier_modules : skiplist.enabled_top_level_modules;
  const topMods = isRelease
    ? rawTopMods.filter((m) => !releaseSkipped.has(m))
    : rawTopMods;

  if (tier === "fast") {
    writeShard("core_fast", topMods.map((m) => `mod ${m};`).join("\n"));
  } else if (isRelease) {
    // In release-balanced, shard top-level modules into small parallel binaries
    // so jsc-link runs concurrently across CPU cores, and include num::cast +
    // iter::{range,traits} (skipping modules in release_skipped_modules).
    const chunkSize = 4;
    for (let i = 0; i < topMods.length; i += chunkSize) {
      const chunk = topMods.slice(i, i + chunkSize);
      writeShard(
        `core_top_${Math.floor(i / chunkSize)}`,
        chunk.map((m) => `mod ${m};`).join("\n"),
      );
    }
    writeShard("core_num_cast", "mod num {\n    mod cast;\n}");
    writeShard(
      "core_iter_range",
      "mod iter {\n    use core::iter::*;\n    pub fn is_trusted_len<I: TrustedLen>(_: I) {}\n    mod range;\n}",
    );
    writeShard(
      "core_iter_traits",
      "mod iter {\n    mod traits { mod accum; mod double_ended; mod step; }\n}",
    );
  } else {
    // Full dev tier: compile & run ALL enabled top-level + fmt/num/iter modules!
    const generalLines = topMods.map((m) => `mod ${m};`);
    if (skiplist.enabled_submodules.fmt?.length) {
      generalLines.push(
        "mod fmt {\n" +
          skiplist.enabled_submodules.fmt.map((sub) => `    mod ${sub};`).join("\n") +
          "\n}",
      );
    }
    writeShard("core_general", generalLines.join("\n"));

    const numHelpers = `    use core::fmt::Debug;
    use core::num::{IntErrorKind, ParseIntError};
    use core::ops::{Add, Div, Mul, Rem, Sub};
    use core::str::FromStr;

    #[macro_use]
    mod int_macros;
    #[macro_use]
    mod uint_macros;

    pub fn test_num<T>(ten: T, two: T)
    where
        T: PartialEq
            + Add<Output = T>
            + Sub<Output = T>
            + Mul<Output = T>
            + Div<Output = T>
            + Rem<Output = T>
            + Debug
            + Copy,
    {
        assert_eq!(ten.add(two), ten + two);
        assert_eq!(ten.sub(two), ten - two);
        assert_eq!(ten.mul(two), ten * two);
        assert_eq!(ten.div(two), ten / two);
        assert_eq!(ten.rem(two), ten % two);
    }

    fn test_parse<T>(num_str: &str, expected: Result<T, IntErrorKind>)
    where
        T: FromStr<Err = ParseIntError>,
        Result<T, IntErrorKind>: PartialEq + Debug,
    {
        assert_eq!(num_str.parse::<T>().map_err(|e| e.kind().clone()), expected)
    }
`;
    const numAll = skiplist.enabled_submodules.num ?? [];
    const uintSubs = numAll.filter((s) => /^u\d+$/.test(s));
    const intSubs = numAll.filter((s) => /^i\d+$/.test(s));
    const castSubs = numAll.filter((s) => s === "cast");
    const miscSubs = numAll.filter(
      (s) => !/^u\d+$/.test(s) && !/^i\d+$/.test(s) && s !== "cast",
    );

    if (uintSubs.length) {
      writeShard(
        "core_num_uint",
        `mod num {\n${numHelpers}\n${uintSubs.map((s) => `    mod ${s};`).join("\n")}\n}`,
      );
    }
    if (intSubs.length) {
      writeShard(
        "core_num_int",
        `mod num {\n${numHelpers}\n${intSubs.map((s) => `    mod ${s};`).join("\n")}\n}`,
      );
    }
    if (castSubs.length) {
      writeShard(
        "core_num_cast",
        `mod num {\n${numHelpers}\n${castSubs.map((s) => `    mod ${s};`).join("\n")}\n}`,
      );
    }
    if (miscSubs.length) {
      writeShard(
        "core_num_misc",
        `mod num {\n${numHelpers}\n${miscSubs.map((s) => `    mod ${s};`).join("\n")}\n}`,
      );
    }

    const iterCfg = skiplist.enabled_submodules.iter;
    if (iterCfg) {
      const iterModSrc = fs.readFileSync(
        path.join(upstreamTestsDir, "iter", "mod.rs"),
        "utf8",
      );
      let iterAdaptersSrc = fs.readFileSync(
        path.join(upstreamTestsDir, "iter", "adapters", "mod.rs"),
        "utf8",
      );
      for (const excl of iterCfg.excluded_adapters ?? []) {
        iterAdaptersSrc = iterAdaptersSrc.replace(`mod ${excl};`, "");
      }
      const traitMods = (iterCfg.enabled_traits ?? []).map((t) => `mod ${t};`).join(" ");
      const patchedIter = iterModSrc
        .replace("mod traits;", `mod traits { ${traitMods} }`)
        .replace("mod adapters;", `mod adapters {\n${iterAdaptersSrc}\n}`);
      writeShard("core_iter", `mod iter {\n${patchedIter}\n}`);
    }
  }

  const skipTests =
    tier === "fast"
      ? Object.keys(skiplist.skipped_tests).filter((t) =>
          topMods.some((m) => t.startsWith(`${m}::`)),
        )
      : Object.keys(skiplist.skipped_tests);

  return {
    manifestPath: path.join(pkgDir, "Cargo.toml"),
    skipTests,
    skippedModulesCount:
      Object.keys(skiplist.skipped_modules).length +
      (isRelease ? releaseSkipped.size : 0),
    skippedTestsCount: skipTests.length,
    enabledModulesSummary:
      tier === "fast"
        ? `${topMods.length} fast modules`
        : isRelease
          ? `${topMods.length} top-level + num::cast + iter::{range,traits}`
          : `${topMods.length} top-level + fmt/num/iter submodules`,
  };
}

function generateAlloctestsHarness(workDir, sysroot, tier, profile) {
  const skiplist = JSON.parse(
    fs.readFileSync(path.join(__dirname, "alloctests", "skiplist.json"), "utf8"),
  );
  const upstreamTestsDir = path.join(
    sysroot,
    "lib",
    "rustlib",
    "src",
    "rust",
    "library",
    "alloctests",
    "tests",
  );
  if (!fs.existsSync(upstreamTestsDir)) {
    throw new Error(
      `Upstream rust-src alloctests not found at ${upstreamTestsDir}. Install component 'rust-src'.`,
    );
  }

  const isRelease = profile === "release-balanced";
  const pkgDir = path.join(workDir, `alloctests-${tier}-${profile}`);
  fs.rmSync(pkgDir, { recursive: true, force: true });
  fs.mkdirSync(path.join(pkgDir, "src"), { recursive: true });
  fs.writeFileSync(
    path.join(pkgDir, "Cargo.toml"),
    `[workspace]\n[package]\nname = "mires_oracle_alloctests"\nversion = "0.1.0"\nedition = "${skiplist.edition}"\n`,
  );
  fs.writeFileSync(path.join(pkgDir, "src", "lib.rs"), "");

  const upstreamLib = fs.readFileSync(path.join(upstreamTestsDir, "lib.rs"), "utf8");
  const headerLines = ["#![allow(warnings)]"];
  for (const line of upstreamLib.split("\n")) {
    if (line.startsWith("mod ")) break;
    headerLines.push(line);
  }
  const commonPrefix = [
    ...headerLines,
    "",
    "fn hash<T: Hash>(t: &T) -> u64 {",
    "    let mut s = DefaultHasher::new();",
    "    t.hash(&mut s);",
    "    s.finish()",
    "}",
    "",
    ...(skiplist.include_testing_helper ? ["mod testing;"] : []),
    "",
  ].join("\n");

  const releaseSkipped = new Set(Object.keys(skiplist.release_skipped_modules ?? {}));
  const rawMods =
    tier === "fast" ? skiplist.fast_tier_modules : skiplist.enabled_modules;
  const topMods = isRelease
    ? rawMods.filter((m) => !releaseSkipped.has(m))
    : rawMods;

  if (isRelease) {
    // One shard per module so jsc-link runs concurrently across CPU cores.
    for (const m of topMods) {
      const shardDir = path.join(pkgDir, "tests", `alloc_${m}`);
      symlinkUpstreamEntries(upstreamTestsDir, shardDir);
      fs.writeFileSync(path.join(shardDir, "main.rs"), `${commonPrefix}mod ${m};\n`);
    }
  } else {
    const shardDir = path.join(pkgDir, "tests", "alloc_all");
    symlinkUpstreamEntries(upstreamTestsDir, shardDir);
    fs.writeFileSync(
      path.join(shardDir, "main.rs"),
      commonPrefix + topMods.map((m) => `mod ${m};`).join("\n") + "\n",
    );
  }

  const skipTests = Object.keys(skiplist.skipped_tests).filter((t) =>
    topMods.some((m) => t.startsWith(`${m}::`)),
  );

  return {
    manifestPath: path.join(pkgDir, "Cargo.toml"),
    skipTests,
    skippedModulesCount:
      Object.keys(skiplist.skipped_modules).length +
      (isRelease ? releaseSkipped.size : 0),
    skippedTestsCount: skipTests.length,
    enabledModulesSummary: `${topMods.length} modules`,
  };
}

function parseCargoMiresOutput(output) {
  const binaries = [];
  let currentBin = null;
  for (const line of output.split("\n")) {
    const runMatch = line.match(/^\s*Running (?:unittests|tests|bin) ([^\s(]+)/);
    if (runMatch) {
      currentBin = runMatch[1];
      continue;
    }
    const resMatch = line.match(
      /^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out/,
    );
    if (resMatch && currentBin) {
      binaries.push({
        name: currentBin,
        status: resMatch[1],
        passed: Number(resMatch[2]),
        failed: Number(resMatch[3]),
        ignored: Number(resMatch[4]),
        filteredOut: Number(resMatch[5]),
      });
      currentBin = null;
    }
  }

  const diffMatch = output.match(
    /Differential check (PASSED|FAILED): (\d+)\/(\d+) tests matched native rustc \((\d+) passed, (\d+) failed, (\d+) ignored\)/,
  );
  const diffFailedMatch = output.match(/error: differential check FAILED \((\d+) mismatch/);

  let passed = binaries.reduce((acc, b) => acc + b.passed, 0);
  let failed = binaries.reduce((acc, b) => acc + b.failed, 0);
  let ignored = binaries.reduce((acc, b) => acc + b.ignored, 0);
  let diffMatched = false;
  let diffTotal = passed + failed + ignored;

  if (diffMatch) {
    diffMatched = diffMatch[1] === "PASSED";
    diffTotal = Number(diffMatch[3]);
    passed = Number(diffMatch[4]);
    failed = Number(diffMatch[5]);
    ignored = Number(diffMatch[6]);
  } else if (diffFailedMatch) {
    diffMatched = false;
  }

  return {
    binaries,
    passed,
    failed,
    ignored,
    diffMatched,
    diffTotal,
  };
}

function runCargoMiresDiff({
  cargoMires,
  manifestPath,
  targetDir,
  profile,
  extraCargoArgs = [],
  harnessArgs = [],
  verbose = false,
}) {
  const args = [
    "mires",
    "test",
    "--diff",
    "--manifest-path",
    manifestPath,
    "--target-dir",
    targetDir,
    "--mires-root",
    REPO_ROOT,
  ];
  if (profile === "release-balanced") {
    args.push("--release");
  }
  args.push(...extraCargoArgs);
  if (harnessArgs.length > 0) {
    args.push("--", ...harnessArgs);
  }

  const env = { ...process.env };
  delete env.CARGO_INCREMENTAL;

  const started = Date.now();
  const res = spawnSync(cargoMires, args, {
    cwd: REPO_ROOT,
    env,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  const durationMs = Date.now() - started;
  const combined = `${res.stdout ?? ""}\n${res.stderr ?? ""}`;

  if (verbose || res.status !== 0) {
    if (res.stdout) process.stdout.write(res.stdout);
    if (res.stderr) process.stderr.write(res.stderr);
  }

  const parsed = parseCargoMiresOutput(combined);
  const ok = res.status === 0 && parsed.failed === 0 && parsed.diffMatched;

  return {
    ok,
    exitCode: res.status ?? 1,
    durationMs,
    ...parsed,
  };
}

function formatDuration(ms) {
  return `${(ms / 1000).toFixed(1)}s`;
}

function renderScorecard(receipt) {
  const lines = [];
  lines.push(`# Mires Continuous Differential Oracle Scorecard`);
  lines.push("");
  lines.push(
    `- **Tier**: \`${receipt.tier}\` | **Toolchain**: \`${receipt.rustcVersion}\` | **Node**: \`${receipt.nodeVersion}\` | **Commit**: \`${receipt.gitSha}\``,
  );
  lines.push(
    `- **Overall**: ${receipt.allPassed ? "**PASS (100% differential parity)**" : "**FAIL**"} — **${receipt.totals.passed} passed**, ${receipt.totals.failed} failed, ${receipt.totals.ignored} ignored across ${receipt.runs.length} suite×profile runs (${formatDuration(receipt.totals.durationMs)})`,
  );
  lines.push("");
  lines.push("## Suite Summary");
  lines.push("");
  lines.push("| Suite | Profile | Status | Diff Parity | Passed | Failed | Ignored | Skipped | Duration |");
  lines.push("|---|---|---|---:|---:|---:|---:|---:|---:|");
  for (const r of receipt.runs) {
    const status = r.ok ? "PASS" : "FAIL";
    const parity = r.diffMatched ? `${r.diffTotal}/${r.diffTotal}` : "MISMATCH";
    const skipped =
      r.skippedModulesCount || r.skippedTestsCount
        ? `${r.skippedModulesCount}m / ${r.skippedTestsCount}t`
        : "0";
    lines.push(
      `| \`${r.suite}\` | \`${r.profile}\` | ${status} | ${parity} | ${r.passed} | ${r.failed} | ${r.ignored} | ${skipped} | ${formatDuration(r.durationMs)} |`,
    );
  }

  const crateRuns = receipt.runs.filter((r) => r.suite === "crates");
  if (crateRuns.length > 0) {
    lines.push("");
    lines.push("## crates.io Breakdown");
    lines.push("");
    const profiles = [...new Set(crateRuns.map((r) => r.profile))];
    const crateNames = [
      ...new Set(
        crateRuns.flatMap((r) => r.binaries.map((b) => b.name)),
      ),
    ].sort();
    lines.push(
      `| Crate | ${profiles.map((p) => `\`${p}\``).join(" | ")} |`,
    );
    lines.push(`|---|${profiles.map(() => "---:").join("|")}|`);
    for (const cname of crateNames) {
      const cells = profiles.map((prof) => {
        const run = crateRuns.find((r) => r.profile === prof);
        const bin = run?.binaries.find((b) => b.name === cname);
        if (!bin) return "-";
        const tag = bin.failed === 0 && bin.status === "ok" ? "PASS" : "FAIL";
        return `${tag} (${bin.passed}/${bin.passed + bin.failed})`;
      });
      lines.push(`| \`${cname}\` | ${cells.join(" | ")} |`);
    }
  }

  lines.push("");
  return lines.join("\n");
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!fs.existsSync(opts.cargoMires)) {
    throw new Error(
      `cargo-mires binary not found at ${opts.cargoMires}. Run 'cargo build --release --locked -p cargo-mires' first.`,
    );
  }

  fs.mkdirSync(opts.outDir, { recursive: true });
  // Keep ephemeral Cargo build artifacts under target/oracle-work (outside build/)
  // so jsc-link's ../build fixture walker never scans thousands of oracle .rlib files.
  const workDir = path.join(REPO_ROOT, "target", "oracle-work");
  fs.mkdirSync(workDir, { recursive: true });

  const sysroot = getRustcSysroot();
  const runs = [];
  const overallStart = Date.now();

  console.log(
    `==> Mires Differential Oracle (tier=${opts.tier}, suites=${opts.suites.join(",")}, profiles=${opts.profiles.join(",")})`,
  );

  // Generate per-profile upstream harnesses lazily inside the loop.
  for (const profile of opts.profiles) {
    for (const suite of opts.suites) {
      let res;
      let skippedModulesCount = 0;
      let skippedTestsCount = 0;

      console.log(`  -> [${profile}] ${suite} ...`);

      if (suite === "rustc-ui") {
        res = runCargoMiresDiff({
          cargoMires: opts.cargoMires,
          manifestPath: path.join(__dirname, "rustc-ui", "Cargo.toml"),
          targetDir: path.join(workDir, "target-rustc-ui"),
          profile,
          extraCargoArgs: ["--tests"],
          verbose: opts.verbose,
        });
      } else if (suite === "coretests") {
        const coretestsHarness = generateCoretestsHarness(
          workDir,
          sysroot,
          opts.tier,
          profile,
        );
        const harnessArgs = [];
        for (const t of coretestsHarness.skipTests) {
          harnessArgs.push("--skip", t);
        }
        skippedModulesCount = coretestsHarness.skippedModulesCount;
        skippedTestsCount = coretestsHarness.skippedTestsCount;
        res = runCargoMiresDiff({
          cargoMires: opts.cargoMires,
          manifestPath: coretestsHarness.manifestPath,
          targetDir: path.join(workDir, `target-coretests-${opts.tier}-${profile}`),
          profile,
          extraCargoArgs: ["--tests"],
          harnessArgs,
          verbose: opts.verbose,
        });
      } else if (suite === "alloctests") {
        const alloctestsHarness = generateAlloctestsHarness(
          workDir,
          sysroot,
          opts.tier,
          profile,
        );
        const harnessArgs = [];
        for (const t of alloctestsHarness.skipTests) {
          harnessArgs.push("--skip", t);
        }
        skippedModulesCount = alloctestsHarness.skippedModulesCount;
        skippedTestsCount = alloctestsHarness.skippedTestsCount;
        res = runCargoMiresDiff({
          cargoMires: opts.cargoMires,
          manifestPath: alloctestsHarness.manifestPath,
          targetDir: path.join(workDir, `target-alloctests-${opts.tier}-${profile}`),
          profile,
          extraCargoArgs: ["--tests"],
          harnessArgs,
          verbose: opts.verbose,
        });
      } else if (suite === "crates") {
        const crateList = opts.tier === "fast" ? FAST_CRATES : ALL_CRATES;
        const extraCargoArgs = [];
        for (const c of crateList) {
          extraCargoArgs.push("--test", c);
        }
        res = runCargoMiresDiff({
          cargoMires: opts.cargoMires,
          manifestPath: path.join(__dirname, "crates", "Cargo.toml"),
          targetDir: path.join(workDir, "target-crates"),
          profile,
          extraCargoArgs,
          verbose: opts.verbose,
        });
      }

      console.log(
        `     ${res.ok ? "PASS" : "FAIL"}: ${res.passed} passed, ${res.failed} failed, ${res.ignored} ignored (diff=${res.diffMatched ? "OK" : "MISMATCH"}, ${formatDuration(res.durationMs)})`,
      );

      runs.push({
        suite,
        profile,
        skippedModulesCount,
        skippedTestsCount,
        ...res,
      });
    }
  }

  const allPassed = runs.every((r) => r.ok);
  const receipt = {
    schemaVersion: 1,
    timestamp: new Date().toISOString(),
    tier: opts.tier,
    gitSha: getGitSha(),
    rustcVersion: getRustcVersion(),
    nodeVersion: process.version,
    allPassed,
    totals: {
      passed: runs.reduce((acc, r) => acc + r.passed, 0),
      failed: runs.reduce((acc, r) => acc + r.failed, 0),
      ignored: runs.reduce((acc, r) => acc + r.ignored, 0),
      durationMs: Date.now() - overallStart,
    },
    runs,
  };

  const receiptPath = path.join(opts.outDir, "receipt.json");
  const scorecardPath = path.join(opts.outDir, "scorecard.md");
  const scorecard = renderScorecard(receipt);

  fs.writeFileSync(receiptPath, JSON.stringify(receipt, null, 2) + "\n");
  fs.writeFileSync(scorecardPath, scorecard);

  console.log("");
  console.log(scorecard);
  console.log(`Wrote ${path.relative(REPO_ROOT, receiptPath)} and ${path.relative(REPO_ROOT, scorecardPath)}`);

  if (!allPassed) {
    process.exit(1);
  }
}

try {
  main();
} catch (err) {
  console.error(`oracle runner error: ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
}
