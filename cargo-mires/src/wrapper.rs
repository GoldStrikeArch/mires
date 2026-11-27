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
pub const ENV_JSC_LINK: &str = "MIRES_WRAPPER_JSC_LINK";
pub const ENV_RECORD_FILE: &str = "MIRES_WRAPPER_RECORD";
pub const ENV_VERBOSE: &str = "MIRES_WRAPPER_VERBOSE";

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
    if rewritten.set_jsc_link {
        cmd.env("JSC_LINK_BIN", &cfg.jsc_link);
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

        let (backend, producer, jsc_link) = if mode == WrapperMode::Build {
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
                if prints_cfg && !has_cfg_mires(&out) {
                    out.push("--cfg".to_owned());
                    out.push("mires".to_owned());
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
            WrapperMode::Build => rewrite_build_target(args, cfg, kind, is_program),
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
    if !has_cfg_mires(&out) {
        out.push("--cfg".to_owned());
        out.push("mires".to_owned());
    }
    if !has_check_cfg_mires(&out) {
        out.push("--check-cfg=cfg(mires)".to_owned());
    }
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

    if !has_cfg_mires(&out) {
        out.push("--cfg".to_owned());
        out.push("mires".to_owned());
    }
    if !has_check_cfg_mires(&out) {
        out.push("--check-cfg=cfg(mires)".to_owned());
    }

    out.push(format!("-Cllvm-args=js-producer={}", cfg.producer));
    for emit_arg in &cfg.emit_args {
        out.push(format!("-Cllvm-args={emit_arg}"));
    }
    if is_program {
        for link_arg in &cfg.link_args {
            out.push(format!("-Cllvm-args={link_arg}"));
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

fn is_program_crate(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
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

fn has_cfg_mires(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--cfg=mires" {
            return true;
        }
        if arg == "--cfg" && args.get(index + 1).map(String::as_str) == Some("mires") {
            return true;
        }
        index += 1;
    }
    false
}

fn has_check_cfg_mires(args: &[String]) -> bool {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--check-cfg=cfg(mires)" {
            return true;
        }
        if arg == "--check-cfg" && args.get(index + 1).map(String::as_str) == Some("cfg(mires)") {
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
}
