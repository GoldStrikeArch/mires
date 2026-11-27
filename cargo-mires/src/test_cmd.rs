//! `cargo mires test` implementation: compile Rust test targets to JS, execute
//! them in Node.js, and optionally diff every test outcome against native `cargo test`.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::cli::{PanicMode, Profile, TestCliOptions};
use crate::metadata;
use crate::sysroot::{self, SysrootRequest, ToolchainInfo};
use crate::wrapper;

/// Node.js module runner invoked for each compiled test binary.
const NODE_TEST_RUNNER_JS: &str = "\
import { pathToFileURL } from 'node:url';
globalThis.__mires_test_args = process.env.MIRES_TEST_ARGS ?? '';
const mod = await import(pathToFileURL(process.argv[1]).href);
if (typeof mod.main !== 'function') {
  console.error('cargo-mires test: compiled test module does not export `main()`');
  process.exit(2);
}
mod.main();
";

#[derive(Debug, Deserialize)]
struct CargoTestMessage {
    reason: String,
    #[serde(default)]
    package_id: String,
    #[serde(default)]
    target: Option<CargoTestTarget>,
    #[serde(default)]
    profile: Option<CargoTestProfile>,
    #[serde(default)]
    executable: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CargoTestTarget {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
    #[serde(default)]
    src_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CargoTestProfile {
    #[serde(default)]
    test: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CompiledTestBinary {
    package_id: String,
    target_name: String,
    target_kind: String,
    src_path: Option<PathBuf>,
    executable: PathBuf,
}

/// Outcome of one individual test case reported by `libtest` or `mires-test`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TestOutcome {
    Ok,
    Failed,
    Ignored,
}

impl TestOutcome {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "FAILED",
            Self::Ignored => "ignored",
        }
    }
}

/// Parsed test results for a single test run (native or Mires).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParsedTestRun {
    /// Map from `(normalized_binary_key, test_name)` to outcome.
    pub outcomes: BTreeMap<(String, String), TestOutcome>,
    pub passed: usize,
    pub failed: usize,
    pub ignored: usize,
    pub filtered_out: usize,
}

/// Execute `cargo mires test`.
pub fn run_test(opts: &TestCliOptions) -> Result<(), String> {
    let started = Instant::now();
    let (profile, panic) = resolve_test_profile_and_panic(opts);
    let toolchain = sysroot::ensure_sysroot(&SysrootRequest {
        mires_root_override: opts.mires_root.as_deref(),
        profile,
        panic,
        clean: false,
        verbose: opts.verbose,
    })?;

    let binaries = compile_mires_tests(opts, &toolchain, profile, panic)?;

    if opts.no_run {
        println!(
            "Compiled {} test target(s) for `{}` (profile={}) in {:.2}s:",
            binaries.len(),
            toolchain.target_name,
            profile,
            started.elapsed().as_secs_f64()
        );
        for bin in &binaries {
            println!("  {} ({})", bin.target_name, bin.executable.display());
        }
        return Ok(());
    }

    if binaries.is_empty() {
        println!("running 0 tests");
        println!();
        println!("test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out");
        return Ok(());
    }

    let (mires_run, mires_ok) = execute_mires_binaries(opts, &binaries)?;

    if opts.diff {
        let (native_run, native_ok) = execute_native_tests(opts, &toolchain.channel, profile)?;
        verify_differential_parity(&native_run, native_ok, &mires_run, mires_ok)?;
    }

    if !mires_ok {
        return Err(format!(
            "test failed ({} passed, {} failed, {} ignored)",
            mires_run.passed, mires_run.failed, mires_run.ignored
        ));
    }

    Ok(())
}

fn resolve_test_profile_and_panic(opts: &TestCliOptions) -> (Profile, PanicMode) {
    let meta = if opts.profile.is_none() || opts.panic.is_none() {
        metadata::try_load_sysroot_metadata(opts.manifest_path.as_deref(), opts.package.as_deref())
    } else {
        None
    };

    let meta_profile = meta
        .as_ref()
        .and_then(|m| m.profile.as_deref())
        .and_then(Profile::parse);
    let meta_panic = meta
        .as_ref()
        .and_then(|m| m.panic.as_deref())
        .and_then(PanicMode::parse);

    let profile = if let Some(explicit) = opts.profile {
        explicit
    } else if opts.release {
        match meta_profile {
            Some(p) if p.is_release() => p,
            _ => Profile::ReleaseBalanced,
        }
    } else {
        Profile::Dev
    };

    let panic = opts.panic.or(meta_panic).unwrap_or(PanicMode::Unwind);
    (profile, panic)
}

fn compile_mires_tests(
    opts: &TestCliOptions,
    toolchain: &ToolchainInfo,
    profile: Profile,
    panic: PanicMode,
) -> Result<Vec<CompiledTestBinary>, String> {
    let wrapper_exe = env::current_exe()
        .map_err(|err| format!("failed to locate current `cargo-mires` executable: {err}"))?;

    let mut emit_args = vec![format!("js-profile={}", profile.as_str())];
    if panic == PanicMode::Unwind {
        emit_args.push("js-cleanup=off".to_owned());
    }
    let link_args = [
        "js-modules=esm",
        "js-runtime=inline",
        "js-dts=off",
        "js-source-map=off",
    ];

    let build_fp = fingerprint(&[
        &toolchain.dylib_sha256,
        &toolchain.sysroot_dir.to_string_lossy(),
        panic.as_str(),
        &emit_args.join("\n"),
        "mode=test",
    ]);

    let rustflags_env = target_rustflags_env_name(&toolchain.target_name);
    let mut target_rustflags = env::var(&rustflags_env).unwrap_or_default();
    if !target_rustflags.is_empty() {
        target_rustflags.push(' ');
    }
    target_rustflags.push_str(&format!(
        "--cfg mires --check-cfg=cfg(mires) --cfg mires_build_fp=\"{build_fp}\""
    ));

    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{}", toolchain.channel)).arg("test");
    if toolchain.is_json_target {
        cmd.arg("-Z").arg("json-target-spec");
    }
    cmd.arg("--no-run")
        .arg("--target")
        .arg(&toolchain.target_arg)
        .arg("--message-format=json-render-diagnostics");

    append_test_cargo_flags(&mut cmd, opts, profile.is_release());

    cmd.env_remove("RUSTUP_TOOLCHAIN")
        .env("RUSTC_WRAPPER", &wrapper_exe)
        .env(&rustflags_env, &target_rustflags)
        .env(wrapper::ENV_WRAPPER_ACTIVE, "1")
        .env(wrapper::ENV_MODE, "test")
        .env(wrapper::ENV_TARGET, &toolchain.target_arg)
        .env(wrapper::ENV_TARGET_NAME, &toolchain.target_name)
        .env(
            wrapper::ENV_JSON_TARGET,
            if toolchain.is_json_target { "1" } else { "0" },
        )
        .env(wrapper::ENV_SYSROOT, &toolchain.sysroot_dir)
        .env(wrapper::ENV_BACKEND, &toolchain.dylib_path)
        .env(wrapper::ENV_PRODUCER, &toolchain.dylib_sha256)
        .env(wrapper::ENV_PANIC, panic.as_str())
        .env(wrapper::ENV_EMIT_ARGS, emit_args.join("\n"))
        .env(wrapper::ENV_LINK_ARGS, link_args.join("\n"))
        .env(wrapper::ENV_JSC_LINK, &toolchain.jsc_link_path)
        .env(wrapper::ENV_VERBOSE, if opts.verbose { "1" } else { "0" })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let output = cmd
        .output()
        .map_err(|err| format!("failed to invoke `cargo test --no-run`: {err}"))?;

    if !output.status.success() {
        return Err(format!(
            "`cargo test --no-run` failed with {}",
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(extract_test_binaries(&stdout))
}

fn append_test_cargo_flags(cmd: &mut Command, opts: &TestCliOptions, release: bool) {
    if let Some(manifest) = &opts.manifest_path {
        cmd.arg("--manifest-path").arg(manifest);
    }
    if opts.workspace {
        cmd.arg("--workspace");
    } else if let Some(pkg) = &opts.package {
        cmd.arg("-p").arg(pkg);
    }
    if release {
        cmd.arg("--release");
    }
    if let Some(target_dir) = &opts.target_dir {
        cmd.arg("--target-dir").arg(target_dir);
    }
    if opts.no_default_features {
        cmd.arg("--no-default-features");
    }
    if opts.all_features {
        cmd.arg("--all-features");
    }
    for feat in &opts.features {
        cmd.arg("--features").arg(feat);
    }
    if opts.targets.lib {
        cmd.arg("--lib");
    }
    if opts.targets.tests {
        cmd.arg("--tests");
    }
    for name in &opts.targets.test_names {
        cmd.arg("--test").arg(name);
    }
    if opts.targets.bins {
        cmd.arg("--bins");
    }
    for name in &opts.targets.bin_names {
        cmd.arg("--bin").arg(name);
    }
}

fn extract_test_binaries(stdout: &str) -> Vec<CompiledTestBinary> {
    let mut binaries = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<CargoTestMessage>(trimmed) else {
            continue;
        };
        if msg.reason != "compiler-artifact" {
            continue;
        }
        let is_test = msg.profile.as_ref().is_some_and(|p| p.test);
        let Some(executable) = msg.executable else {
            continue;
        };
        if !is_test {
            continue;
        }
        let (target_name, target_kind, src_path) = match msg.target {
            Some(t) => (
                t.name,
                t.kind.first().cloned().unwrap_or_else(|| "test".to_owned()),
                t.src_path,
            ),
            None => ("test".to_owned(), "test".to_owned(), None),
        };
        binaries.push(CompiledTestBinary {
            package_id: msg.package_id,
            target_name,
            target_kind,
            src_path,
            executable,
        });
    }
    binaries
}

fn execute_mires_binaries(
    opts: &TestCliOptions,
    binaries: &[CompiledTestBinary],
) -> Result<(ParsedTestRun, bool), String> {
    let mut combined_args: Vec<String> = Vec::new();
    combined_args.extend(opts.filters.iter().cloned());
    combined_args.extend(opts.harness_args.iter().cloned());
    let encoded_args = combined_args.join("\u{001f}");

    let mut aggregate = ParsedTestRun::default();
    let mut all_ok = true;

    for bin in binaries {
        let mjs_path = stage_mjs_module(&bin.executable)?;
        let label = format_binary_banner(bin);
        println!("     Running {label} ({})", mjs_path.display());

        let output = Command::new("node")
            .arg("--input-type=module")
            .arg("-e")
            .arg(NODE_TEST_RUNNER_JS)
            .arg(&mjs_path)
            .env("MIRES_TEST_ARGS", &encoded_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|err| {
                format!(
                    "failed to execute Node.js for `{}` (ensure `node` is installed in PATH): {err}",
                    mjs_path.display()
                )
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stdout.is_empty() {
            print!("{stdout}");
        }
        if !stderr.is_empty() {
            eprint!("{stderr}");
        }

        if !output.status.success() {
            all_ok = false;
        }

        let bin_key = normalize_binary_key(&bin.target_name);
        parse_libtest_output_into(&stdout, &bin_key, &mut aggregate);
    }

    Ok((aggregate, all_ok))
}

fn stage_mjs_module(executable: &Path) -> Result<PathBuf, String> {
    let mut os = executable.as_os_str().to_os_string();
    os.push(".mjs");
    let mjs_path = PathBuf::from(os);
    fs::copy(executable, &mjs_path).map_err(|err| {
        format!(
            "failed to stage test module `{}` -> `{}`: {err}",
            executable.display(),
            mjs_path.display()
        )
    })?;
    Ok(mjs_path)
}

fn format_binary_banner(bin: &CompiledTestBinary) -> String {
    let kind_label = if bin.target_kind == "test" {
        "tests"
    } else {
        "unittests"
    };
    match &bin.src_path {
        Some(path) => {
            let tail = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&bin.target_name);
            format!("{kind_label} {} ({tail})", bin.target_name)
        }
        None => format!("{kind_label} {}", bin.target_name),
    }
}

fn execute_native_tests(
    opts: &TestCliOptions,
    channel: &str,
    profile: Profile,
) -> Result<(ParsedTestRun, bool), String> {
    println!("==> Running native `cargo test` for differential comparison");
    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{channel}"))
        .arg("test")
        .arg("--no-fail-fast");
    append_test_cargo_flags(&mut cmd, opts, profile.is_release());
    // Skip doctests during differential comparison unless specifically requested,
    // because `cargo mires test` compiles unit/integration/bin test harnesses.
    let explicit_target = opts.targets.lib
        || opts.targets.tests
        || opts.targets.bins
        || !opts.targets.test_names.is_empty()
        || !opts.targets.bin_names.is_empty();
    if !explicit_target {
        cmd.arg("--lib").arg("--bins").arg("--tests");
    }

    let has_test_threads = opts
        .harness_args
        .iter()
        .any(|a| a == "--test-threads" || a.starts_with("--test-threads="));

    if !opts.filters.is_empty() || !opts.harness_args.is_empty() || !has_test_threads {
        cmd.arg("--");
        for filter in &opts.filters {
            cmd.arg(filter);
        }
        for arg in &opts.harness_args {
            cmd.arg(arg);
        }
        if !has_test_threads {
            cmd.arg("--test-threads=1");
        }
    }

    cmd.env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC_WRAPPER")
        .env_remove(wrapper::ENV_WRAPPER_ACTIVE)
        .env_remove(wrapper::ENV_LINK_WRAPPER)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd
        .output()
        .map_err(|err| format!("failed to run native `cargo test`: {err}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if opts.verbose && !stderr.is_empty() {
        eprint!("{stderr}");
    }

    let parsed = parse_native_cargo_test_output(&stdout, &stderr);
    Ok((parsed, output.status.success()))
}

fn verify_differential_parity(
    native: &ParsedTestRun,
    native_ok: bool,
    mires: &ParsedTestRun,
    mires_ok: bool,
) -> Result<(), String> {
    let mut mismatches: Vec<String> = Vec::new();

    for (key, native_outcome) in &native.outcomes {
        match mires.outcomes.get(key) {
            Some(mires_outcome) if mires_outcome == native_outcome => {}
            Some(mires_outcome) => {
                mismatches.push(format!(
                    "  {}::{}: native={} vs mires={}",
                    key.0,
                    key.1,
                    native_outcome.as_str(),
                    mires_outcome.as_str()
                ));
            }
            None => {
                mismatches.push(format!(
                    "  {}::{}: native={} vs mires=MISSING",
                    key.0,
                    key.1,
                    native_outcome.as_str()
                ));
            }
        }
    }

    for (key, mires_outcome) in &mires.outcomes {
        if !native.outcomes.contains_key(key) {
            mismatches.push(format!(
                "  {}::{}: native=MISSING vs mires={}",
                key.0,
                key.1,
                mires_outcome.as_str()
            ));
        }
    }

    if native_ok != mires_ok {
        mismatches.push(format!(
            "  exit status mismatch: native_ok={native_ok} vs mires_ok={mires_ok}"
        ));
    }

    if !mismatches.is_empty() {
        return Err(format!(
            "differential check FAILED ({} mismatch(es)):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        ));
    }

    println!(
        "Differential check PASSED: {}/{} tests matched native rustc ({} passed, {} failed, {} ignored)",
        mires.outcomes.len(),
        native.outcomes.len(),
        mires.passed,
        mires.failed,
        mires.ignored
    );
    Ok(())
}

/// Parse native `cargo test` output (tracking `Running ... (<exe>)` headers from stderr/stdout).
pub fn parse_native_cargo_test_output(stdout: &str, stderr: &str) -> ParsedTestRun {
    // Cargo writes `Running ... (target/debug/deps/<name>-<hash>)` on stderr and
    // test lines on stdout; extract ordered binary keys from stderr and map each
    // `running N tests` section in stdout to its corresponding binary key.
    let headers = extract_running_binary_keys(stderr);
    let mut run = ParsedTestRun::default();
    let mut section_idx = 0usize;
    let mut current_key = headers
        .first()
        .cloned()
        .unwrap_or_else(|| "default".to_owned());
    let mut saw_first_section = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("running ")
            && (trimmed.ends_with(" test") || trimmed.ends_with(" tests"))
        {
            if saw_first_section {
                section_idx += 1;
                current_key = headers
                    .get(section_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("target_{section_idx}"));
            } else {
                saw_first_section = true;
            }
            continue;
        }
        parse_single_libtest_line(trimmed, &current_key, &mut run);
    }

    run
}

fn extract_running_binary_keys(stderr: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for line in stderr.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("Running ") {
            continue;
        }
        let Some(open) = trimmed.rfind('(') else {
            continue;
        };
        let Some(close) = trimmed.rfind(')') else {
            continue;
        };
        if close <= open + 1 {
            continue;
        }
        let path_str = &trimmed[open + 1..close];
        let file_stem = Path::new(path_str)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(path_str);
        // Strip Cargo's `-<16hex>` hash suffix from `deps/<name>-<16hex>`.
        let base = strip_cargo_hash_suffix(file_stem);
        keys.push(normalize_binary_key(base));
    }
    keys
}

fn strip_cargo_hash_suffix(stem: &str) -> &str {
    if let Some((prefix, suffix)) = stem.rsplit_once('-') {
        if suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return prefix;
        }
    }
    stem
}

fn normalize_binary_key(name: &str) -> String {
    name.replace('-', "_")
}

/// Parse standard `libtest` output for a single test binary into `run`.
pub fn parse_libtest_output_into(stdout: &str, bin_key: &str, run: &mut ParsedTestRun) {
    for line in stdout.lines() {
        parse_single_libtest_line(line.trim(), bin_key, run);
    }
}

fn parse_single_libtest_line(trimmed: &str, bin_key: &str, run: &mut ParsedTestRun) {
    if let Some(rest) = trimmed.strip_prefix("test ") {
        if let Some((lhs, rhs)) = rest.rsplit_once(" ... ") {
            let name = lhs.strip_suffix(" - should panic").unwrap_or(lhs).trim();
            let outcome = if rhs == "ok" {
                Some(TestOutcome::Ok)
            } else if rhs == "FAILED" {
                Some(TestOutcome::Failed)
            } else if rhs == "ignored" || rhs.starts_with("ignored,") {
                Some(TestOutcome::Ignored)
            } else {
                None
            };
            if let Some(outcome) = outcome {
                run.outcomes
                    .insert((bin_key.to_owned(), name.to_owned()), outcome);
            }
            return;
        }
    }

    if let Some(rest) = trimmed.strip_prefix("test result: ") {
        if let Some((_, counts)) = rest.split_once(". ") {
            for part in counts.split(';') {
                let p = part.trim();
                if let Some(num) = p.strip_suffix(" passed") {
                    if let Ok(n) = num.trim().parse::<usize>() {
                        run.passed += n;
                    }
                } else if let Some(num) = p.strip_suffix(" failed") {
                    if let Ok(n) = num.trim().parse::<usize>() {
                        run.failed += n;
                    }
                } else if let Some(num) = p.strip_suffix(" ignored") {
                    if let Ok(n) = num.trim().parse::<usize>() {
                        run.ignored += n;
                    }
                } else if let Some(num) = p.strip_suffix(" filtered out") {
                    if let Ok(n) = num.trim().parse::<usize>() {
                        run.filtered_out += n;
                    }
                }
            }
        }
    }
}

fn target_rustflags_env_name(target_name: &str) -> String {
    let upper: String = target_name
        .chars()
        .map(|c| match c {
            '-' | '.' => '_',
            other => other.to_ascii_uppercase(),
        })
        .collect();
    format!("CARGO_TARGET_{upper}_RUSTFLAGS")
}

fn fingerprint(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    let hex = format!("{:x}", hasher.finalize());
    hex[..16].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libtest_output_and_summary_counts() {
        let out = "\n\
running 4 tests\n\
test a::basic ... ok\n\
test a::panics - should panic ... ok\n\
test a::later ... ignored, todo\n\
test a::broken ... FAILED\n\
\n\
test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 3 filtered out; finished in 0.01s\n";

        let mut run = ParsedTestRun::default();
        parse_libtest_output_into(out, "my_crate", &mut run);

        assert_eq!(run.passed, 2);
        assert_eq!(run.failed, 1);
        assert_eq!(run.ignored, 1);
        assert_eq!(run.filtered_out, 3);
        assert_eq!(
            run.outcomes
                .get(&("my_crate".to_owned(), "a::basic".to_owned())),
            Some(&TestOutcome::Ok)
        );
        assert_eq!(
            run.outcomes
                .get(&("my_crate".to_owned(), "a::panics".to_owned())),
            Some(&TestOutcome::Ok)
        );
        assert_eq!(
            run.outcomes
                .get(&("my_crate".to_owned(), "a::later".to_owned())),
            Some(&TestOutcome::Ignored)
        );
        assert_eq!(
            run.outcomes
                .get(&("my_crate".to_owned(), "a::broken".to_owned())),
            Some(&TestOutcome::Failed)
        );
    }

    #[test]
    fn parses_multi_binary_native_output_and_diffs_parity() {
        let stderr = "\
   Compiling demo v0.1.0\n\
     Running unittests src/lib.rs (target/debug/deps/my_crate-0123456789abcdef)\n\
     Running tests/integ.rs (target/debug/deps/integ-fedcba9876543210)\n";
        let stdout = "\n\
running 1 test\n\
test unit_ok ... ok\n\
\n\
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\
\n\
running 1 test\n\
test integ_ok ... ok\n\
\n\
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";

        let native = parse_native_cargo_test_output(stdout, stderr);
        assert_eq!(native.passed, 2);
        assert_eq!(
            native
                .outcomes
                .get(&("my_crate".to_owned(), "unit_ok".to_owned())),
            Some(&TestOutcome::Ok)
        );
        assert_eq!(
            native
                .outcomes
                .get(&("integ".to_owned(), "integ_ok".to_owned())),
            Some(&TestOutcome::Ok)
        );

        assert!(verify_differential_parity(&native, true, &native, true).is_ok());

        let mut broken = native.clone();
        broken.outcomes.insert(
            ("integ".to_owned(), "integ_ok".to_owned()),
            TestOutcome::Failed,
        );
        assert!(verify_differential_parity(&native, true, &broken, false).is_err());
    }
}
