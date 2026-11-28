//! `RUSTC_WRAPPER` engine for intercepting Cargo's `rustc` invocations.
//!
//! When `MIRES_RUSTC_WRAPPER=1` is set in the environment, `cargo-mires` runs as
//! `cargo-mires <rustc_path> <rustc_args...>` and classifies each call into:
//! - Cargo probe invocations (`-vV`, `--version`, `--print=...`),
//! - Host crate compilations (`build.rs`, proc-macros — no matching `--target`),
//! - Target crate compilations (matching `--target`, handled for `check` or `build`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::cli::PanicMode;

pub const ENV_WRAPPER_ACTIVE: &str = "MIRES_RUSTC_WRAPPER";
pub const ENV_LINK_WRAPPER: &str = "MIRES_LINK_WRAPPER";
pub const ENV_REAL_JSC_LINK: &str = "MIRES_REAL_JSC_LINK";
pub const ENV_TEST_CRATE_NAME: &str = "MIRES_TEST_CRATE_NAME";
pub const ENV_MODE: &str = "MIRES_WRAPPER_MODE";
pub const ENV_TARGET: &str = "MIRES_WRAPPER_TARGET";
pub const ENV_TARGET_NAME: &str = "MIRES_WRAPPER_TARGET_NAME";
pub const ENV_JSON_TARGET: &str = "MIRES_WRAPPER_JSON_TARGET";
pub const ENV_SYSROOT: &str = "MIRES_WRAPPER_SYSROOT";
pub const ENV_BACKEND: &str = "MIRES_WRAPPER_BACKEND";
pub const ENV_PRODUCER: &str = "MIRES_WRAPPER_PRODUCER";
pub const ENV_PANIC: &str = "MIRES_WRAPPER_PANIC";
pub const ENV_EMIT_ARGS: &str = "MIRES_WRAPPER_EMIT_ARGS";
pub const ENV_LINK_ARGS: &str = "MIRES_WRAPPER_LINK_ARGS";
pub const ENV_CFGS: &str = "MIRES_WRAPPER_CFGS";
pub const ENV_JSC_LINK: &str = "MIRES_WRAPPER_JSC_LINK";
pub const ENV_RECORD_FILE: &str = "MIRES_WRAPPER_RECORD";
pub const ENV_VERBOSE: &str = "MIRES_WRAPPER_VERBOSE";

/// Placeholder `js-root` rewritten by `run_link_wrapper` to the test binary's `main` JS symbol.
const TEST_MAIN_ROOT_SENTINEL: &str = "__MIRES_TEST_MAIN__";

/// Conflicting `-C <opt>` keys stripped from Cargo's arguments before injecting Mires flags.
const CONFLICTING_CODEGEN_KEYS: [&str; 6] = [
    "codegen-units",
    "debuginfo",
    "panic",
    "overflow-checks",
    "debug-assertions",
    "lto",
];

/// Wrapper operating mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WrapperMode {
    Build,
    Check,
    Test,
}

/// Configuration passed from `cargo-mires` parent process to the wrapper via environment variables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WrapperConfig {
    pub mode: WrapperMode,
    pub target: String,
    pub target_name: String,
    pub is_json_target: bool,
    pub sysroot: PathBuf,
    pub backend: PathBuf,
    pub producer: String,
    pub panic: PanicMode,
    pub emit_args: Vec<String>,
    pub link_args: Vec<String>,
    pub cfgs: Vec<String>,
    pub jsc_link: PathBuf,
    pub record_file: Option<PathBuf>,
    pub verbose: bool,
}

/// High-level classification of a single `rustc` invocation made by Cargo.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationKind {
    Probe {
        target_matches: bool,
        prints_cfg: bool,
    },
    Host,
    Target {
        is_program: bool,
    },
}

/// Predicted raw (`outputs.path(Exe)`) and Cargo-expected (`.wasm`) program output paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgramOutputPaths {
    pub raw_output: PathBuf,
    pub cargo_expected: PathBuf,
}

/// Result of rewriting Cargo's `rustc` argument list.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewrittenInvocation {
    pub kind: InvocationKind,
    pub args: Vec<String>,
    pub set_jsc_link: bool,
    pub program_output: Option<ProgramOutputPaths>,
}

/// Entry point when invoked by Cargo via `RUSTC_WRAPPER`.
pub fn run() -> ExitCode {
    let mut raw = env::args_os();
    let _wrapper_exe = raw.next();
    let Some(rustc_exe) = raw.next() else {
        eprintln!("cargo-mires (wrapper): missing rustc path argument");
        return ExitCode::from(2);
    };
    let args: Vec<String> = raw.map(|arg| arg.to_string_lossy().into_owned()).collect();

    let cfg = match WrapperConfig::from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("cargo-mires (wrapper): {err}");
            return ExitCode::from(2);
        }
    };

    let rewritten = rewrite_args(&args, &cfg);

    if cfg.verbose {
        eprintln!(
            "cargo-mires (wrapper) [{:?}]: {} {}",
            rewritten.kind,
            Path::new(&rustc_exe).display(),
            rewritten.args.join(" ")
        );
    }

    let mut cmd = Command::new(&rustc_exe);
    cmd.args(&rewritten.args);
    cmd.env_remove(ENV_WRAPPER_ACTIVE);
    if rewritten.set_jsc_link {
        let use_test_link_wrapper = cfg.mode == WrapperMode::Test && has_test_flag(&args);
        if use_test_link_wrapper {
            match env::current_exe() {
                Ok(self_exe) => {
                    cmd.env("JSC_LINK_BIN", &self_exe)
                        .env(ENV_LINK_WRAPPER, "1")
                        .env(ENV_REAL_JSC_LINK, &cfg.jsc_link);
                    if let Some(crate_name) = extract_crate_name(&args) {
                        cmd.env(ENV_TEST_CRATE_NAME, crate_name);
                    }
                }
                Err(err) => {
                    eprintln!("cargo-mires (wrapper): failed to locate self exe: {err}");
                    return ExitCode::FAILURE;
                }
            }
        } else {
            cmd.env("JSC_LINK_BIN", &cfg.jsc_link);
        }
    }

    let status = match cmd.status() {
        Ok(status) => status,
        Err(err) => {
            eprintln!(
                "cargo-mires (wrapper): failed to execute `{}`: {err}",
                Path::new(&rustc_exe).display()
            );
            return ExitCode::FAILURE;
        }
    };

    if status.success() {
        if let Some(paths) = &rewritten.program_output {
            let actual_raw = bridge_program_output(paths);
            if let (Some(record_file), Some(raw_path)) = (&cfg.record_file, actual_raw) {
                let _ = fs::write(record_file, raw_path.to_string_lossy().as_bytes());
            }
        }
    }

    match status.code() {
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    }
}

impl WrapperConfig {
    fn from_env() -> Result<Self, String> {
        let mode = match env::var(ENV_MODE).as_deref() {
            Ok("build") => WrapperMode::Build,
            Ok("check") => WrapperMode::Check,
            Ok("test") => WrapperMode::Test,
            Ok(other) => return Err(format!("invalid `{ENV_MODE}`: `{other}`")),
            Err(_) => return Err(format!("missing `{ENV_MODE}`")),
        };
        let target = required_env(ENV_TARGET)?;
        let target_name = required_env(ENV_TARGET_NAME)?;
        let is_json_target = env::var(ENV_JSON_TARGET).as_deref() == Ok("1");
        let sysroot = PathBuf::from(required_env(ENV_SYSROOT)?);
        let panic_raw = required_env(ENV_PANIC)?;
        let panic = PanicMode::parse(&panic_raw)
            .ok_or_else(|| format!("invalid `{ENV_PANIC}`: `{panic_raw}`"))?;

        let (backend, producer, jsc_link) =
            if matches!(mode, WrapperMode::Build | WrapperMode::Test) {
                (
                    PathBuf::from(required_env(ENV_BACKEND)?),
                    required_env(ENV_PRODUCER)?,
                    PathBuf::from(required_env(ENV_JSC_LINK)?),
                )
            } else {
                (PathBuf::new(), String::new(), PathBuf::new())
            };

        let emit_args = parse_lines_env(ENV_EMIT_ARGS);
        let link_args = parse_lines_env(ENV_LINK_ARGS);
        let cfgs = parse_lines_env(ENV_CFGS);
        let record_file = env::var_os(ENV_RECORD_FILE).map(PathBuf::from);
        let verbose = env::var(ENV_VERBOSE).as_deref() == Ok("1");

        Ok(Self {
            mode,
            target,
            target_name,
            is_json_target,
            sysroot,
            backend,
            producer,
            panic,
            emit_args,
            link_args,
            cfgs,
            jsc_link,
            record_file,
            verbose,
        })
    }
}

fn required_env(key: &str) -> Result<String, String> {
    env::var(key).map_err(|_| format!("missing environment variable `{key}`"))
}

fn parse_lines_env(key: &str) -> Vec<String> {
    env::var(key)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Classify a `rustc` invocation as a Cargo probe, host crate build, or target crate build.
pub fn classify_invocation(args: &[String], cfg: &WrapperConfig) -> InvocationKind {
    let target_matches = extract_target(args)
        .is_some_and(|target| matches_mires_target(target, &cfg.target, &cfg.target_name));

    let (is_probe, prints_cfg) = inspect_probe_flags(args);
    if is_probe {
        return InvocationKind::Probe {
            target_matches,
            prints_cfg,
        };
    }

    if !target_matches {
        return InvocationKind::Host;
    }

    let is_program = is_program_crate(args);
    InvocationKind::Target { is_program }
}

/// Rewrite Cargo's `rustc` argument list according to the wrapper rules.
pub fn rewrite_args(args: &[String], cfg: &WrapperConfig) -> RewrittenInvocation {
    let kind = classify_invocation(args, cfg);
    match kind {
        InvocationKind::Host => RewrittenInvocation {
            kind,
            args: args.to_vec(),
            set_jsc_link: false,
            program_output: None,
        },
        InvocationKind::Probe {
            target_matches,
            prints_cfg,
        } => {
            let mut out = args.to_vec();
            if target_matches {
                if cfg.is_json_target && !has_unstable_options(&out) {
                    out.push("-Zunstable-options".to_owned());
                }
                if prints_cfg {
                    if !has_cfg(&out, "mires") {
                        out.push("--cfg".to_owned());
                        out.push("mires".to_owned());
                    }
                    for spec in &cfg.cfgs {
                        if !has_cfg(&out, spec) {
                            out.push("--cfg".to_owned());
                            out.push(spec.clone());
                        }
                    }
                }
            }
            RewrittenInvocation {
                kind,
                args: out,
                set_jsc_link: false,
                program_output: None,
            }
        }
        InvocationKind::Target { is_program } => match cfg.mode {
            WrapperMode::Check => rewrite_check_target(args, cfg, kind),
            WrapperMode::Build | WrapperMode::Test => {
                rewrite_build_target(args, cfg, kind, is_program)
            }
        },
    }
}

fn rewrite_check_target(
    args: &[String],
    cfg: &WrapperConfig,
    kind: InvocationKind,
) -> RewrittenInvocation {
    let mut out = filter_args(args, &["panic"]);
    if cfg.is_json_target && !has_unstable_options(&out) {
        out.push("-Zunstable-options".to_owned());
    }
    out.push(format!("--sysroot={}", cfg.sysroot.display()));
    out.push(format!("-Cpanic={}", cfg.panic.as_str()));
    inject_cfgs(&mut out, &cfg.cfgs);
    RewrittenInvocation {
        kind,
        args: out,
        set_jsc_link: false,
        program_output: None,
    }
}

fn rewrite_build_target(
    args: &[String],
    cfg: &WrapperConfig,
    kind: InvocationKind,
    is_program: bool,
) -> RewrittenInvocation {
    let program_output = if is_program {
        predict_program_output(args)
    } else {
        None
    };

    let mut out = filter_args(args, &CONFLICTING_CODEGEN_KEYS);
    if cfg.is_json_target && !has_unstable_options(&out) {
        out.push("-Zunstable-options".to_owned());
    }
    out.push(format!("-Zcodegen-backend={}", cfg.backend.display()));
    out.push(format!("--sysroot={}", cfg.sysroot.display()));
    out.push("-Ccodegen-units=1".to_owned());
    out.push(format!("-Cpanic={}", cfg.panic.as_str()));
    out.push("-Coverflow-checks=off".to_owned());
    out.push("-Cdebug-assertions=off".to_owned());
    out.push("-Cdebuginfo=0".to_owned());

    inject_cfgs(&mut out, &cfg.cfgs);

    out.push(format!("-Cllvm-args=js-producer={}", cfg.producer));
    for emit_arg in &cfg.emit_args {
        out.push(format!("-Cllvm-args={emit_arg}"));
    }
    if is_program {
        for link_arg in &cfg.link_args {
            out.push(format!("-Cllvm-args={link_arg}"));
        }
        if cfg.mode == WrapperMode::Test && has_test_flag(args) {
            out.push(format!("-Cllvm-args=js-root={TEST_MAIN_ROOT_SENTINEL}"));
        }
    }

    RewrittenInvocation {
        kind,
        args: out,
        set_jsc_link: true,
        program_output,
    }
}

fn extract_target(args: &[String]) -> Option<&str> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if let Some(value) = arg.strip_prefix("--target=") {
            return Some(value);
        }
        if arg == "--target" {
            return args.get(index + 1).map(String::as_str);
        }
        index += 1;
    }
    None
}

fn matches_mires_target(target: &str, expected_target: &str, expected_name: &str) -> bool {
    if target == expected_target || target == expected_name {
        return true;
    }
    if target.ends_with(".json") {
        let stem = Path::new(target).file_stem().and_then(|s| s.to_str());
        if stem == Some(expected_name) {
            return true;
        }
    }
    false
}

fn inspect_probe_flags(args: &[String]) -> (bool, bool) {
    let mut is_probe = false;
    let mut prints_cfg = false;
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if matches!(arg, "-vV" | "--version") {
            is_probe = true;
        } else if let Some(what) = arg.strip_prefix("--print=") {
            is_probe = true;
            if what == "cfg" {
                prints_cfg = true;
            }
        } else if arg == "--print" {
            is_probe = true;
            if args.get(index + 1).map(String::as_str) == Some("cfg") {
                prints_cfg = true;
            }
            index += 1;
        }
        index += 1;
    }
    (is_probe, prints_cfg)
}

fn has_test_flag(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--test")
}

fn is_program_crate(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--test" {
            return true;
        }
        let kind = if let Some(value) = arg.strip_prefix("--crate-type=") {
            Some(value)
        } else if arg == "--crate-type" {
            index += 1;
            args.get(index).map(String::as_str)
        } else {
            None
        };
        if let Some(kinds) = kind {
            for item in kinds.split(',') {
                if matches!(item, "cdylib" | "bin" | "dylib") {
                    return true;
                }
            }
        }
        index += 1;
    }
    false
}

fn extract_crate_name(args: &[String]) -> Option<&str> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if let Some(name) = arg.strip_prefix("--crate-name=") {
            return Some(name);
        }
        if arg == "--crate-name" {
            return args.get(index + 1).map(String::as_str);
        }
        index += 1;
    }
    None
}

/// Entry point when `rustc_codegen_mires` invokes us as `JSC_LINK_BIN` for a test binary.
///
/// Resolves the test harness `main` symbol in the compiled CGU object, patches the
/// sentinel `js-root` inside the `jsc-link` manifest, invokes the real `jsc-link`,
/// and appends `export function main()` so Node.js can execute the test ESM module.
pub fn run_link_wrapper() -> ExitCode {
    let args: Vec<String> = env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let Ok(real_jsc_link) = env::var(ENV_REAL_JSC_LINK) else {
        eprintln!("cargo-mires (link-wrapper): missing `{ENV_REAL_JSC_LINK}`");
        return ExitCode::from(2);
    };

    let manifest_path = find_manifest_arg(&args);
    let crate_name = env::var(ENV_TEST_CRATE_NAME).ok();

    let linked_target = if let Some(ref mp) = manifest_path {
        match prepare_test_manifest(Path::new(&real_jsc_link), mp, crate_name.as_deref()) {
            Ok(target) => target,
            Err(err) => {
                eprintln!("cargo-mires (link-wrapper): {err}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };

    let mut cmd = Command::new(&real_jsc_link);
    cmd.args(&args)
        .env_remove(ENV_LINK_WRAPPER)
        .env_remove(ENV_WRAPPER_ACTIVE);
    let status = match cmd.status() {
        Ok(status) => status,
        Err(err) => {
            eprintln!("cargo-mires (link-wrapper): failed to execute `{real_jsc_link}`: {err}");
            return ExitCode::FAILURE;
        }
    };

    if !status.success() {
        return match status.code() {
            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
            None => ExitCode::FAILURE,
        };
    }

    if let Some((out_file, main_sym)) = linked_target {
        if let Err(err) = append_test_main_export(&out_file, &main_sym) {
            eprintln!("cargo-mires (link-wrapper): {err}");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

fn find_manifest_arg(args: &[String]) -> Option<PathBuf> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--manifest" {
            return args.get(index + 1).map(PathBuf::from);
        }
        index += 1;
    }
    None
}

fn prepare_test_manifest(
    real_jsc_link: &Path,
    manifest_path: &Path,
    crate_name: Option<&str>,
) -> Result<Option<(PathBuf, String)>, String> {
    let printed = Command::new(real_jsc_link)
        .arg("--print-manifest")
        .arg(manifest_path)
        .env_remove(ENV_LINK_WRAPPER)
        .env_remove(ENV_WRAPPER_ACTIVE)
        .output()
        .map_err(|err| format!("failed to run `jsc-link --print-manifest`: {err}"))?;
    if !printed.status.success() {
        return Err(format!(
            "`jsc-link --print-manifest` failed with {}",
            printed.status
        ));
    }
    let text = String::from_utf8_lossy(&printed.stdout);
    let objects = parse_manifest_objects(&text);
    let out_file = parse_manifest_output(&text);

    let main_sym = find_main_symbol_in_objects(&objects, crate_name).ok_or_else(|| {
        format!(
            "failed to locate test `main` symbol in CGU objects ({})",
            objects
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    let raw = fs::read(manifest_path)
        .map_err(|err| format!("failed to read `{}`: {err}", manifest_path.display()))?;
    let patched = replace_postcard_string(&raw, TEST_MAIN_ROOT_SENTINEL, &main_sym)
        .ok_or_else(|| "test root sentinel not found in `jsc-link` manifest".to_owned())?;
    fs::write(manifest_path, patched)
        .map_err(|err| format!("failed to write patched manifest: {err}"))?;

    Ok(out_file.map(|path| (path, main_sym)))
}

fn parse_manifest_objects(printed: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Some(start) = printed.find("objects: [") else {
        return out;
    };
    let rest = &printed[start + "objects: [".len()..];
    let Some(end) = rest.find(']') else {
        return out;
    };
    for line in rest[..end].lines() {
        let trimmed = line.trim().trim_end_matches(',');
        if let Some(unquoted) = parse_debug_string(trimmed) {
            out.push(PathBuf::from(unquoted));
        }
    }
    out
}

fn parse_manifest_output(printed: &str) -> Option<PathBuf> {
    let idx = printed.find("output: Some(")?;
    let rest = &printed[idx + "output: Some(".len()..];
    let end = rest.find(')')?;
    let inner = rest[..end].trim().trim_end_matches(',').trim();
    parse_debug_string(inner).map(PathBuf::from)
}

fn parse_debug_string(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next()? {
                '\\' => out.push('\\'),
                '"' => out.push('"'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
        } else {
            out.push(ch);
        }
    }
    Some(out)
}

fn find_main_symbol_in_objects(objects: &[PathBuf], crate_name: Option<&str>) -> Option<String> {
    let expected_prefix = crate_name.map(|name| format!("{name}$main$h"));
    let mut fallback: Option<String> = None;

    for path in objects {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for candidate in scan_function_main_symbols(&text) {
            if let Some(ref prefix) = expected_prefix {
                if candidate.starts_with(prefix) {
                    return Some(candidate);
                }
            }
            if fallback.is_none() {
                fallback = Some(candidate);
            }
        }
    }
    fallback
}

/// Scans raw CGU object text for top-level `function <crate>$main$h<16hex>(` declarations.
pub fn scan_function_main_symbols(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    let needle = "function ";
    let mut offset = 0;
    while let Some(pos) = text[offset..].find(needle) {
        let start = offset + pos + needle.len();
        let rest = &text[start..];
        if let Some(paren) = rest.find('(') {
            let ident = &rest[..paren];
            if is_crate_main_symbol(ident) {
                results.push(ident.to_owned());
            }
        }
        offset = start;
    }
    results
}

fn is_crate_main_symbol(ident: &str) -> bool {
    let Some((prefix, hash)) = ident.rsplit_once("$main$h") else {
        return false;
    };
    !prefix.is_empty()
        && !prefix.contains('$')
        && prefix
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && hash.len() == 16
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Replaces one postcard length-prefixed UTF-8 string inside raw manifest bytes.
pub fn replace_postcard_string(bytes: &[u8], old: &str, new: &str) -> Option<Vec<u8>> {
    let old_enc = encode_postcard_str(old);
    let new_enc = encode_postcard_str(new);
    let pos = bytes.windows(old_enc.len()).position(|w| w == old_enc)?;
    let mut out = Vec::with_capacity(bytes.len() - old_enc.len() + new_enc.len());
    out.extend_from_slice(&bytes[..pos]);
    out.extend_from_slice(&new_enc);
    out.extend_from_slice(&bytes[pos + old_enc.len()..]);
    Some(out)
}

fn encode_postcard_str(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() + 5);
    let mut n = s.len();
    while n >= 0x80 {
        out.push(((n & 0x7f) as u8) | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
    out.extend_from_slice(s.as_bytes());
    out
}

fn append_test_main_export(out_file: &Path, main_sym: &str) -> Result<(), String> {
    let text = fs::read_to_string(out_file)
        .map_err(|err| format!("failed to read `{}`: {err}", out_file.display()))?;
    if text.contains("export function main(") {
        return Ok(());
    }
    let emitted_ident = resolve_emitted_main_ident(&text, main_sym).ok_or_else(|| {
        format!(
            "failed to resolve emitted `main` identifier (expected `{main_sym}` or caller of `__mires_test_main_static`) in `{}`",
            out_file.display()
        )
    })?;
    let mut updated = text;
    if !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(&format!(
        "export function main() {{ return {emitted_ident}(); }}\n"
    ));
    fs::write(out_file, updated.as_bytes())
        .map_err(|err| format!("failed to write `{}`: {err}", out_file.display()))
}

/// Resolves the final emitted JS identifier for `main` (handling both unminified and
/// `js-minify=on` release outputs where `main_sym` was shortened).
pub fn resolve_emitted_main_ident(js: &str, main_sym: &str) -> Option<String> {
    if js.contains(&format!("function {main_sym}(")) {
        return Some(main_sym.to_owned());
    }
    // Under `js-minify=on`, `main_sym` is shortened, but `__mires_test_main_static`
    // is `#[no_mangle]` so its call site inside `main` preserves its exact name.
    let call_needle = "__mires_test_main_static(";
    let mut search_from = 0;
    while let Some(rel) = js[search_from..].find(call_needle) {
        let abs = search_from + rel;
        let is_def = js[..abs].ends_with("function ");
        if !is_def {
            if let Some(fn_pos) = js[..abs].rfind("function ") {
                let after_kw = &js[fn_pos + "function ".len()..abs];
                if let Some(paren) = after_kw.find("()") {
                    let ident = after_kw[..paren].trim();
                    if is_js_ident(ident) && ident != "__mires_test_main_static" {
                        return Some(ident.to_owned());
                    }
                }
            }
        }
        search_from = abs + call_needle.len();
    }
    None
}

fn is_js_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let valid_start = first.is_ascii_alphabetic() || first == '_' || first == '$';
    valid_start && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Remove conflicting `-C` flags (both `-Ckey=val` and `-C key=val`) and synthetic
/// Cargo fingerprint cfgs (`mires_build_fp=...`, `mires_link_fp=...`).
fn filter_args(args: &[String], blocked_cg_keys: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "-C" {
            if let Some(next) = args.get(index + 1) {
                if is_blocked_cg_value(next, blocked_cg_keys) {
                    index += 2;
                    continue;
                }
            }
        } else if let Some(rest) = arg.strip_prefix("-C") {
            if !rest.is_empty() && is_blocked_cg_value(rest, blocked_cg_keys) {
                index += 1;
                continue;
            }
        } else if arg == "--cfg" {
            if let Some(next) = args.get(index + 1) {
                if is_synthetic_fingerprint_cfg(next) {
                    index += 2;
                    continue;
                }
            }
        } else if let Some(rest) = arg.strip_prefix("--cfg=") {
            if is_synthetic_fingerprint_cfg(rest) {
                index += 1;
                continue;
            }
        }

        out.push(args[index].clone());
        index += 1;
    }
    out
}

fn is_blocked_cg_value(value: &str, blocked_cg_keys: &[&str]) -> bool {
    let key = value.split_once('=').map_or(value, |(k, _)| k);
    blocked_cg_keys.contains(&key)
}

fn is_synthetic_fingerprint_cfg(value: &str) -> bool {
    value.starts_with("mires_build_fp=") || value.starts_with("mires_link_fp=")
}

fn has_unstable_options(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "-Zunstable-options" {
            return true;
        }
        if arg == "-Z" && args.get(index + 1).map(String::as_str) == Some("unstable-options") {
            return true;
        }
        index += 1;
    }
    false
}

fn inject_cfgs(out: &mut Vec<String>, cfgs: &[String]) {
    if !has_cfg(out, "mires") {
        out.push("--cfg".to_owned());
        out.push("mires".to_owned());
    }
    if !has_check_cfg(out, "cfg(mires)") {
        out.push("--check-cfg=cfg(mires)".to_owned());
    }
    for spec in cfgs {
        if !has_cfg(out, spec) {
            out.push("--cfg".to_owned());
            out.push(spec.clone());
        }
        if let Some(expr) = crate::cli::check_cfg_expr(spec) {
            if !has_check_cfg(out, &expr) {
                out.push(format!("--check-cfg={expr}"));
            }
        }
    }
}

fn has_cfg(args: &[String], spec: &str) -> bool {
    let inline = format!("--cfg={spec}");
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == inline {
            return true;
        }
        if arg == "--cfg" && args.get(index + 1).map(String::as_str) == Some(spec) {
            return true;
        }
        index += 1;
    }
    false
}

fn has_check_cfg(args: &[String], expr: &str) -> bool {
    let inline = format!("--check-cfg={expr}");
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == inline {
            return true;
        }
        if arg == "--check-cfg" && args.get(index + 1).map(String::as_str) == Some(expr) {
            return true;
        }
        index += 1;
    }
    false
}

/// Bridge `backend/src/link.rs`'s `outputs.path(OutputType::Exe)` (no `.wasm` suffix)
/// to Cargo's expected `<name>.wasm` filename when Cargo invoked `rustc --out-dir`.
fn bridge_program_output(paths: &ProgramOutputPaths) -> Option<PathBuf> {
    if paths.raw_output != paths.cargo_expected && paths.raw_output.is_file() {
        let _ = fs::copy(&paths.raw_output, &paths.cargo_expected);
        return Some(paths.raw_output.clone());
    }
    if paths.cargo_expected.is_file() {
        return Some(paths.cargo_expected.clone());
    }
    None
}

/// Predict the exact output paths rustc/jsc-link writes and Cargo expects for a program crate.
fn predict_program_output(args: &[String]) -> Option<ProgramOutputPaths> {
    let mut out_dir: Option<&str> = None;
    let mut crate_name: Option<&str> = None;
    let mut extra_filename: &str = "";

    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "-o" {
            let path = PathBuf::from(args.get(index + 1)?);
            return Some(ProgramOutputPaths {
                raw_output: path.clone(),
                cargo_expected: path,
            });
        }
        if let Some(path) = arg.strip_prefix("-o") {
            if !path.is_empty() {
                let path = PathBuf::from(path);
                return Some(ProgramOutputPaths {
                    raw_output: path.clone(),
                    cargo_expected: path,
                });
            }
        }
        if arg == "--out-dir" {
            out_dir = args.get(index + 1).map(String::as_str);
            index += 2;
            continue;
        }
        if let Some(dir) = arg.strip_prefix("--out-dir=") {
            out_dir = Some(dir);
        } else if arg == "--crate-name" {
            crate_name = args.get(index + 1).map(String::as_str);
            index += 2;
            continue;
        } else if let Some(name) = arg.strip_prefix("--crate-name=") {
            crate_name = Some(name);
        } else if arg == "-C" {
            if let Some(val) = args.get(index + 1) {
                if let Some(ext) = val.strip_prefix("extra-filename=") {
                    extra_filename = ext;
                }
            }
            index += 2;
            continue;
        } else if let Some(ext) = arg.strip_prefix("-Cextra-filename=") {
            extra_filename = ext;
        }
        index += 1;
    }

    let dir = Path::new(out_dir?);
    let name = crate_name?;
    Some(ProgramOutputPaths {
        raw_output: dir.join(format!("{name}{extra_filename}")),
        cargo_expected: dir.join(format!("{name}{extra_filename}.wasm")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config(mode: WrapperMode) -> WrapperConfig {
        WrapperConfig {
            mode,
            target: "/mires/targets/mires32-unwind.json".to_owned(),
            target_name: "mires32-unwind".to_owned(),
            is_json_target: true,
            sysroot: PathBuf::from("/mires/build/sysroots/abc123"),
            backend: PathBuf::from("/mires/target/release/librustc_codegen_mires.dylib"),
            producer: "0123456789abcdef".repeat(4),
            panic: PanicMode::Unwind,
            emit_args: vec![
                "js-profile=release-balanced".to_owned(),
                "js-cleanup=off".to_owned(),
            ],
            link_args: vec![
                "js-modules=esm".to_owned(),
                "js-runtime=inline".to_owned(),
                "js-dts=on".to_owned(),
                "js-source-map=on".to_owned(),
            ],
            cfgs: Vec::new(),
            jsc_link: PathBuf::from("/mires/target/release/jsc-link"),
            record_file: None,
            verbose: false,
        }
    }

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn host_probe_v_version_passes_through_without_backend() {
        let cfg = sample_config(WrapperMode::Build);
        let args = strs(&["-vV"]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(
            res.kind,
            InvocationKind::Probe {
                target_matches: false,
                prints_cfg: false
            }
        );
        assert_eq!(res.args, args);
        assert!(!res.set_jsc_link);
    }

    #[test]
    fn target_probe_adds_cfg_mires_and_unstable_options_but_no_backend() {
        let cfg = sample_config(WrapperMode::Build);
        let args = strs(&[
            "-",
            "--crate-name",
            "___",
            "--print=file-names",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "--print=sysroot",
            "--print=cfg",
        ]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(
            res.kind,
            InvocationKind::Probe {
                target_matches: true,
                prints_cfg: true
            }
        );
        assert!(res.args.contains(&"-Zunstable-options".to_owned()));
        assert!(res.args.windows(2).any(|w| w == ["--cfg", "mires"]));
        assert!(!res.args.iter().any(|a| a.starts_with("-Zcodegen-backend=")));
        assert!(!res.args.iter().any(|a| a.starts_with("--sysroot")));
        assert!(!res.set_jsc_link);
    }

    #[test]
    fn host_crate_and_proc_macro_compile_completely_unmodified() {
        let cfg = sample_config(WrapperMode::Build);
        let args = strs(&[
            "--crate-name",
            "serde_derive",
            "--crate-type",
            "proc-macro",
            "-C",
            "codegen-units=16",
            "-C",
            "debuginfo=2",
            "src/lib.rs",
        ]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(res.kind, InvocationKind::Host);
        assert_eq!(res.args, args);
        assert!(!res.set_jsc_link);
    }

    #[test]
    fn target_rlib_filters_conflicting_codegen_flags_and_omits_link_only_args() {
        let cfg = sample_config(WrapperMode::Build);
        let args = strs(&[
            "--crate-name",
            "my_dep",
            "--crate-type",
            "rlib",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "-C",
            "codegen-units=16",
            "-Cdebuginfo=2",
            "-C",
            "panic=abort",
            "-Coverflow-checks=on",
            "-C",
            "debug-assertions=on",
            "-Clto=thin",
            "-C",
            "lto",
            "-C",
            "metadata=deadbeef",
            "--cfg",
            "mires_build_fp=\"1234\"",
            "src/lib.rs",
        ]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(res.kind, InvocationKind::Target { is_program: false });
        assert!(res.set_jsc_link);
        assert_eq!(res.program_output, None);

        // Conflicting Cargo flags were stripped
        assert!(!res.args.iter().any(|a| a == "codegen-units=16"));
        assert!(!res.args.iter().any(|a| a == "-Cdebuginfo=2"));
        assert!(!res.args.iter().any(|a| a == "panic=abort"));
        assert!(!res.args.iter().any(|a| a == "-Coverflow-checks=on"));
        assert!(!res.args.iter().any(|a| a == "debug-assertions=on"));
        assert!(!res.args.iter().any(|a| a.contains("lto")));
        assert!(!res.args.iter().any(|a| a.starts_with("mires_build_fp=")));

        // Non-conflicting Cargo metadata preserved
        assert!(res
            .args
            .windows(2)
            .any(|w| w == ["-C", "metadata=deadbeef"]));

        // Required Mires flags injected
        assert!(res.args.contains(
            &"-Zcodegen-backend=/mires/target/release/librustc_codegen_mires.dylib".to_owned()
        ));
        assert!(res
            .args
            .contains(&"--sysroot=/mires/build/sysroots/abc123".to_owned()));
        assert!(res.args.contains(&"-Ccodegen-units=1".to_owned()));
        assert!(res.args.contains(&"-Cpanic=unwind".to_owned()));
        assert!(res.args.contains(&"-Coverflow-checks=off".to_owned()));
        assert!(res.args.contains(&"-Cdebug-assertions=off".to_owned()));
        assert!(res.args.contains(&"-Cdebuginfo=0".to_owned()));
        assert!(res
            .args
            .contains(&"-Cllvm-args=js-profile=release-balanced".to_owned()));
        assert!(res.args.contains(&"-Cllvm-args=js-cleanup=off".to_owned()));

        // Link-only args NOT passed to rlib
        assert!(!res.args.contains(&"-Cllvm-args=js-modules=esm".to_owned()));
        assert!(!res
            .args
            .contains(&"-Cllvm-args=js-runtime=inline".to_owned()));
    }

    #[test]
    fn target_cdylib_receives_link_args_and_predicts_output_path() {
        let cfg = sample_config(WrapperMode::Build);
        let args = strs(&[
            "--crate-name",
            "my_app",
            "--crate-type",
            "cdylib",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "--out-dir",
            "/ws/target/mires32-unwind/release/deps",
            "-C",
            "extra-filename=-99aa",
            "src/lib.rs",
        ]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(res.kind, InvocationKind::Target { is_program: true });
        assert!(res.set_jsc_link);
        assert!(res.args.contains(&"-Cllvm-args=js-modules=esm".to_owned()));
        assert!(res
            .args
            .contains(&"-Cllvm-args=js-runtime=inline".to_owned()));
        assert!(res.args.contains(&"-Cllvm-args=js-dts=on".to_owned()));
        assert!(res
            .args
            .contains(&"-Cllvm-args=js-source-map=on".to_owned()));
        assert_eq!(
            res.program_output,
            Some(ProgramOutputPaths {
                raw_output: PathBuf::from("/ws/target/mires32-unwind/release/deps/my_app-99aa"),
                cargo_expected: PathBuf::from(
                    "/ws/target/mires32-unwind/release/deps/my_app-99aa.wasm"
                ),
            })
        );
    }

    #[test]
    fn test_mode_marks_dash_dash_test_as_program_and_injects_sentinel_root() {
        let cfg = sample_config(WrapperMode::Test);
        let args = strs(&[
            "--crate-name",
            "shared",
            "--test",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "--out-dir",
            "/ws/target/mires32-unwind/debug/deps",
            "-C",
            "extra-filename=-1122",
            "src/lib.rs",
        ]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(res.kind, InvocationKind::Target { is_program: true });
        assert!(res
            .args
            .contains(&format!("-Cllvm-args=js-root={TEST_MAIN_ROOT_SENTINEL}")));
    }

    #[test]
    fn scans_main_symbol_patches_postcard_and_resolves_minified_main() {
        let raw_obj = "function other$foo$h0123456789abcdef() {}\n\
                       function my_pkg$main$h0123456789abcdef() {}\n\
                       function my_pkg$sub$main$hfedcba9876543210() {}\n";
        assert_eq!(
            scan_function_main_symbols(raw_obj),
            vec!["my_pkg$main$h0123456789abcdef".to_owned()]
        );

        let mut manifest_bytes = vec![0x05, 0x00, 0x01];
        manifest_bytes.extend_from_slice(&encode_postcard_str(TEST_MAIN_ROOT_SENTINEL));
        manifest_bytes.extend_from_slice(&[0xaa, 0xbb]);
        let patched = replace_postcard_string(
            &manifest_bytes,
            TEST_MAIN_ROOT_SENTINEL,
            "my_pkg$main$h0123456789abcdef",
        )
        .unwrap();
        let mut expected = vec![0x05, 0x00, 0x01];
        expected.extend_from_slice(&encode_postcard_str("my_pkg$main$h0123456789abcdef"));
        expected.extend_from_slice(&[0xaa, 0xbb]);
        assert_eq!(patched, expected);

        let unminified =
            "function my_pkg$main$h0123456789abcdef() { __mires_test_main_static([]); }\n";
        assert_eq!(
            resolve_emitted_main_ident(unminified, "my_pkg$main$h0123456789abcdef").as_deref(),
            Some("my_pkg$main$h0123456789abcdef")
        );

        let minified =
            "function __mires_test_main_static(a){}\nfunction $q(){__mires_test_main_static(x);}\n";
        assert_eq!(
            resolve_emitted_main_ident(minified, "my_pkg$main$h0123456789abcdef").as_deref(),
            Some("$q")
        );

        let printed_manifest = "\
Manifest {\n\
    mode: Program {\n\
        output: Some(\n\
            \"/tmp/shared-97a95c05a0a07541.wasm\",\n\
        ),\n\
    },\n\
    objects: [\n\
        \"/tmp/shared.cgu.0.rcgu.o\",\n\
    ],\n\
}\n";
        assert_eq!(
            parse_manifest_output(printed_manifest),
            Some(PathBuf::from("/tmp/shared-97a95c05a0a07541.wasm"))
        );
        assert_eq!(
            parse_manifest_objects(printed_manifest),
            vec![PathBuf::from("/tmp/shared.cgu.0.rcgu.o")]
        );
    }

    #[test]
    fn check_mode_omits_codegen_backend_and_keeps_sysroot_and_cfg_mires() {
        let cfg = sample_config(WrapperMode::Check);
        let args = strs(&[
            "--crate-name",
            "my_app",
            "--crate-type",
            "lib",
            "--emit=dep-info,metadata",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "src/lib.rs",
        ]);
        let res = rewrite_args(&args, &cfg);
        assert_eq!(res.kind, InvocationKind::Target { is_program: false });
        assert!(!res.set_jsc_link);
        assert!(!res.args.iter().any(|a| a.starts_with("-Zcodegen-backend=")));
        assert!(res
            .args
            .contains(&"--sysroot=/mires/build/sysroots/abc123".to_owned()));
        assert!(res.args.contains(&"-Cpanic=unwind".to_owned()));
        assert!(res.args.windows(2).any(|w| w == ["--cfg", "mires"]));
        assert!(res.args.contains(&"--check-cfg=cfg(mires)".to_owned()));
    }

    #[test]
    fn custom_cfgs_forward_to_probes_check_and_build_without_duplication() {
        let mut cfg = sample_config(WrapperMode::Build);
        cfg.cfgs = vec!["topcoat_client".to_owned(), "mode=\"csr\"".to_owned()];

        let probe_args = strs(&[
            "-",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "--print=cfg",
        ]);
        let probe_res = rewrite_args(&probe_args, &cfg);
        assert!(probe_res
            .args
            .windows(2)
            .any(|w| w == ["--cfg", "topcoat_client"]));
        assert!(probe_res
            .args
            .windows(2)
            .any(|w| w == ["--cfg", "mode=\"csr\""]));

        let target_args = strs(&[
            "--crate-name",
            "app",
            "--crate-type",
            "cdylib",
            "--target",
            "/mires/targets/mires32-unwind.json",
            "--cfg",
            "topcoat_client",
            "--check-cfg=cfg(topcoat_client)",
            "src/lib.rs",
        ]);
        let build_res = rewrite_args(&target_args, &cfg);
        assert_eq!(
            build_res
                .args
                .windows(2)
                .filter(|w| *w == ["--cfg", "topcoat_client"])
                .count(),
            1
        );
        assert!(build_res
            .args
            .windows(2)
            .any(|w| w == ["--cfg", "mode=\"csr\""]));
        assert!(build_res
            .args
            .contains(&"--check-cfg=cfg(mode,values(any()))".to_owned()));

        cfg.mode = WrapperMode::Check;
        let check_res = rewrite_args(&target_args, &cfg);
        assert!(check_res
            .args
            .windows(2)
            .any(|w| w == ["--cfg", "mode=\"csr\""]));
        assert!(check_res
            .args
            .contains(&"--check-cfg=cfg(mode,values(any()))".to_owned()));
    }
}
