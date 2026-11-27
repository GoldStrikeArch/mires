//! Command-line argument parsing and core configuration types for `cargo-mires`.

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

/// Compiler optimization and output profile (`js-profile=<name>`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Profile {
    Dev,
    Report,
    ReleaseSize,
    ReleaseBalanced,
    ReleaseSpeed,
}

impl Profile {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Report => "report",
            Self::ReleaseSize => "release-size",
            Self::ReleaseBalanced => "release-balanced",
            Self::ReleaseSpeed => "release-speed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dev" => Some(Self::Dev),
            "report" => Some(Self::Report),
            "release-size" => Some(Self::ReleaseSize),
            "release-balanced" => Some(Self::ReleaseBalanced),
            "release-speed" => Some(Self::ReleaseSpeed),
            _ => None,
        }
    }

    pub const fn is_release(self) -> bool {
        matches!(
            self,
            Self::ReleaseSize | Self::ReleaseBalanced | Self::ReleaseSpeed
        )
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Output JavaScript format (`js-modules=<esm|script>`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Esm,
    Script,
}

impl OutputFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Esm => "esm",
            Self::Script => "script",
        }
    }

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Esm => "mjs",
            Self::Script => "js",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "esm" => Some(Self::Esm),
            "script" => Some(Self::Script),
            _ => None,
        }
    }
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Runtime shim delivery strategy (`js-runtime=<inline|external|none>`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeMode {
    Inline,
    External,
    None,
}

impl RuntimeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::External => "external",
            Self::None => "none",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "inline" => Some(Self::Inline),
            "external" => Some(Self::External),
            "none" => Some(Self::None),
            _ => None,
        }
    }
}

impl fmt::Display for RuntimeMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Target panic strategy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PanicMode {
    Unwind,
    Abort,
}

impl PanicMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unwind => "unwind",
            Self::Abort => "abort",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unwind" => Some(Self::Unwind),
            "abort" => Some(Self::Abort),
            _ => None,
        }
    }

    pub const fn target_name(self) -> &'static str {
        match self {
            Self::Unwind => "mires32-unwind",
            Self::Abort => "wasm32-unknown-unknown",
        }
    }
}

impl fmt::Display for PanicMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Explicit Cargo target selection from CLI flags.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum TargetSelection {
    #[default]
    Auto,
    Lib,
    Bin(String),
}

/// CLI options shared by `build`, `check`, and `watch`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuildCliOptions {
    pub package: Option<String>,
    pub manifest_path: Option<PathBuf>,
    pub release: bool,
    pub profile: Option<Profile>,
    pub format: Option<OutputFormat>,
    pub runtime: Option<RuntimeMode>,
    pub out_dir: Option<PathBuf>,
    pub dts: Option<bool>,
    pub source_map: Option<bool>,
    pub npm: Option<bool>,
    pub panic: Option<PanicMode>,
    pub mires_root: Option<PathBuf>,
    pub features: Vec<String>,
    pub all_features: bool,
    pub no_default_features: bool,
    pub target_dir: Option<PathBuf>,
    pub target_selection: TargetSelection,
    pub verbose: bool,
}

/// CLI options for `cargo mires sysroot`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SysrootCliOptions {
    pub clean: bool,
    pub release: bool,
    pub profile: Option<Profile>,
    pub panic: Option<PanicMode>,
    pub mires_root: Option<PathBuf>,
    pub manifest_path: Option<PathBuf>,
    pub package: Option<String>,
    pub verbose: bool,
}

/// CLI options for `cargo mires setup`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SetupCliOptions {
    pub version: Option<String>,
    pub from_archive: Option<PathBuf>,
    pub url: Option<String>,
    pub sha256: Option<String>,
    pub no_rustup: bool,
    pub force: bool,
    pub verbose: bool,
}

/// CLI options for `cargo mires toolchain path`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolchainPathOptions {
    pub mires_root: Option<PathBuf>,
}

/// CLI options for `cargo mires toolchain clean`.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolchainCleanOptions {
    pub all: bool,
    pub version: Option<String>,
}

/// CLI options for `cargo mires toolchain package`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolchainPackageOptions {
    pub out: PathBuf,
    pub mires_root: Option<PathBuf>,
    pub version: Option<String>,
    pub host: Option<String>,
    pub include_cli: bool,
    pub no_sysroots: bool,
    pub verbose: bool,
}

/// Subcommands under `cargo mires toolchain`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolchainCliCommand {
    List,
    Path(ToolchainPathOptions),
    Clean(ToolchainCleanOptions),
    Package(ToolchainPackageOptions),
}

/// Parsed top-level command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Build(BuildCliOptions),
    Check(BuildCliOptions),
    Watch(BuildCliOptions),
    Sysroot(SysrootCliOptions),
    Setup(SetupCliOptions),
    Toolchain(ToolchainCliCommand),
    Help(Option<String>),
    Version,
}

/// Parse process arguments (including `argv[0]` and an optional Cargo `mires` subcommand token).
pub fn parse_args<I, T>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let raw: Vec<String> = args
        .into_iter()
        .skip(1)
        .map(|arg| arg.into().to_string_lossy().into_owned())
        .collect();

    let slice = match raw.first().map(String::as_str) {
        Some("mires") => &raw[1..],
        _ => &raw[..],
    };

    let Some((first, rest)) = slice.split_first() else {
        return Ok(Command::Help(None));
    };

    match first.as_str() {
        "-h" | "--help" => Ok(Command::Help(None)),
        "help" => Ok(Command::Help(rest.first().cloned())),
        "-V" | "--version" => Ok(Command::Version),
        "build" => parse_build_like(rest).map(|parsed| match parsed {
            ParsedSubcommand::Help => Command::Help(Some("build".to_owned())),
            ParsedSubcommand::Opts(opts) => Command::Build(opts),
        }),
        "check" => parse_build_like(rest).map(|parsed| match parsed {
            ParsedSubcommand::Help => Command::Help(Some("check".to_owned())),
            ParsedSubcommand::Opts(opts) => Command::Check(opts),
        }),
        "watch" => parse_build_like(rest).map(|parsed| match parsed {
            ParsedSubcommand::Help => Command::Help(Some("watch".to_owned())),
            ParsedSubcommand::Opts(opts) => Command::Watch(opts),
        }),
        "sysroot" => parse_sysroot(rest).map(|parsed| match parsed {
            ParsedSubcommand::Help => Command::Help(Some("sysroot".to_owned())),
            ParsedSubcommand::Opts(opts) => Command::Sysroot(opts),
        }),
        "setup" => parse_setup(rest).map(|parsed| match parsed {
            ParsedSubcommand::Help => Command::Help(Some("setup".to_owned())),
            ParsedSubcommand::Opts(opts) => Command::Setup(opts),
        }),
        "toolchain" => parse_toolchain(rest).map(|parsed| match parsed {
            ParsedSubcommand::Help => Command::Help(Some("toolchain".to_owned())),
            ParsedSubcommand::Opts(cmd) => Command::Toolchain(cmd),
        }),
        other => Err(format!(
            "unknown subcommand `{other}` (expected `build`, `check`, `watch`, `sysroot`, `setup`, or `toolchain`)"
        )),
    }
}

enum ParsedSubcommand<T> {
    Help,
    Opts(T),
}

fn parse_build_like(args: &[String]) -> Result<ParsedSubcommand<BuildCliOptions>, String> {
    let mut opts = BuildCliOptions::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }

        let (flag, inline_value) = split_flag(arg);
        match flag {
            "-p" | "--package" => {
                opts.package = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "--manifest-path" => {
                opts.manifest_path = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--release" => {
                reject_value(flag, inline_value)?;
                opts.release = true;
            }
            "--profile" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                let profile = Profile::parse(&raw).ok_or_else(|| {
                    format!(
                        "invalid value `{raw}` for `--profile` \
                         (expected `dev`, `report`, `release-size`, `release-balanced`, or `release-speed`)"
                    )
                })?;
                opts.profile = Some(profile);
            }
            "--format" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                let format = OutputFormat::parse(&raw).ok_or_else(|| {
                    format!("invalid value `{raw}` for `--format` (expected `esm` or `script`)")
                })?;
                opts.format = Some(format);
            }
            "--runtime" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                let runtime = RuntimeMode::parse(&raw).ok_or_else(|| {
                    format!(
                        "invalid value `{raw}` for `--runtime` (expected `inline`, `external`, or `none`)"
                    )
                })?;
                opts.runtime = Some(runtime);
            }
            "--out-dir" => {
                opts.out_dir = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--dts" => {
                reject_value(flag, inline_value)?;
                opts.dts = Some(true);
            }
            "--no-dts" => {
                reject_value(flag, inline_value)?;
                opts.dts = Some(false);
            }
            "--source-map" => {
                reject_value(flag, inline_value)?;
                opts.source_map = Some(true);
            }
            "--no-source-map" => {
                reject_value(flag, inline_value)?;
                opts.source_map = Some(false);
            }
            "--npm" => {
                reject_value(flag, inline_value)?;
                opts.npm = Some(true);
            }
            "--no-npm" => {
                reject_value(flag, inline_value)?;
                opts.npm = Some(false);
            }
            "--panic" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                let panic = PanicMode::parse(&raw).ok_or_else(|| {
                    format!("invalid value `{raw}` for `--panic` (expected `unwind` or `abort`)")
                })?;
                opts.panic = Some(panic);
            }
            "--mires-root" => {
                opts.mires_root = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "-F" | "--features" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                opts.features.push(raw);
            }
            "--all-features" => {
                reject_value(flag, inline_value)?;
                opts.all_features = true;
            }
            "--no-default-features" => {
                reject_value(flag, inline_value)?;
                opts.no_default_features = true;
            }
            "--target-dir" => {
                opts.target_dir = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--lib" => {
                reject_value(flag, inline_value)?;
                if matches!(opts.target_selection, TargetSelection::Bin(_)) {
                    return Err("cannot pass both `--lib` and `--bin`".to_owned());
                }
                opts.target_selection = TargetSelection::Lib;
            }
            "--bin" => {
                let name = take_value(args, &mut index, flag, inline_value)?;
                if opts.target_selection == TargetSelection::Lib {
                    return Err("cannot pass both `--lib` and `--bin`".to_owned());
                }
                opts.target_selection = TargetSelection::Bin(name);
            }
            "-v" | "--verbose" => {
                reject_value(flag, inline_value)?;
                opts.verbose = true;
            }
            other => {
                return Err(format!("unknown option `{other}`"));
            }
        }
        index += 1;
    }

    Ok(ParsedSubcommand::Opts(opts))
}

fn parse_sysroot(args: &[String]) -> Result<ParsedSubcommand<SysrootCliOptions>, String> {
    let mut opts = SysrootCliOptions::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }

        let (flag, inline_value) = split_flag(arg);
        match flag {
            "--clean" => {
                reject_value(flag, inline_value)?;
                opts.clean = true;
            }
            "--release" => {
                reject_value(flag, inline_value)?;
                opts.release = true;
            }
            "--profile" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                let profile = Profile::parse(&raw).ok_or_else(|| {
                    format!(
                        "invalid value `{raw}` for `--profile` \
                         (expected `dev`, `report`, `release-size`, `release-balanced`, or `release-speed`)"
                    )
                })?;
                opts.profile = Some(profile);
            }
            "--panic" => {
                let raw = take_value(args, &mut index, flag, inline_value)?;
                let panic = PanicMode::parse(&raw).ok_or_else(|| {
                    format!("invalid value `{raw}` for `--panic` (expected `unwind` or `abort`)")
                })?;
                opts.panic = Some(panic);
            }
            "--mires-root" => {
                opts.mires_root = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--manifest-path" => {
                opts.manifest_path = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "-p" | "--package" => {
                opts.package = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "-v" | "--verbose" => {
                reject_value(flag, inline_value)?;
                opts.verbose = true;
            }
            other => {
                return Err(format!("unknown option `{other}` for `sysroot`"));
            }
        }
        index += 1;
    }

    Ok(ParsedSubcommand::Opts(opts))
}

fn parse_setup(args: &[String]) -> Result<ParsedSubcommand<SetupCliOptions>, String> {
    let mut opts = SetupCliOptions::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }

        let (flag, inline_value) = split_flag(arg);
        match flag {
            "--version" => {
                opts.version = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "--from-archive" => {
                opts.from_archive = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--url" => {
                opts.url = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "--sha256" => {
                opts.sha256 = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "--no-rustup" => {
                reject_value(flag, inline_value)?;
                opts.no_rustup = true;
            }
            "--force" => {
                reject_value(flag, inline_value)?;
                opts.force = true;
            }
            "-v" | "--verbose" => {
                reject_value(flag, inline_value)?;
                opts.verbose = true;
            }
            other => {
                return Err(format!("unknown option `{other}` for `setup`"));
            }
        }
        index += 1;
    }

    Ok(ParsedSubcommand::Opts(opts))
}

fn parse_toolchain(args: &[String]) -> Result<ParsedSubcommand<ToolchainCliCommand>, String> {
    let Some((sub, rest)) = args.split_first() else {
        return Ok(ParsedSubcommand::Help);
    };

    match sub.as_str() {
        "-h" | "--help" | "help" => Ok(ParsedSubcommand::Help),
        "list" => parse_toolchain_list(rest),
        "path" => parse_toolchain_path(rest),
        "clean" => parse_toolchain_clean(rest),
        "package" => parse_toolchain_package(rest),
        other => Err(format!(
            "unknown `toolchain` subcommand `{other}` (expected `list`, `path`, `clean`, or `package`)"
        )),
    }
}

fn parse_toolchain_list(args: &[String]) -> Result<ParsedSubcommand<ToolchainCliCommand>, String> {
    if let Some(arg) = args.first() {
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }
        return Err(format!("unknown option `{arg}` for `toolchain list`"));
    }
    Ok(ParsedSubcommand::Opts(ToolchainCliCommand::List))
}

fn parse_toolchain_path(args: &[String]) -> Result<ParsedSubcommand<ToolchainCliCommand>, String> {
    let mut opts = ToolchainPathOptions::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }
        let (flag, inline_value) = split_flag(arg);
        match flag {
            "--mires-root" => {
                opts.mires_root = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            other => {
                return Err(format!("unknown option `{other}` for `toolchain path`"));
            }
        }
        index += 1;
    }

    Ok(ParsedSubcommand::Opts(ToolchainCliCommand::Path(opts)))
}

fn parse_toolchain_clean(args: &[String]) -> Result<ParsedSubcommand<ToolchainCliCommand>, String> {
    let mut opts = ToolchainCleanOptions::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }
        let (flag, inline_value) = split_flag(arg);
        match flag {
            "--all" => {
                reject_value(flag, inline_value)?;
                opts.all = true;
            }
            "--version" => {
                opts.version = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            other => {
                return Err(format!("unknown option `{other}` for `toolchain clean`"));
            }
        }
        index += 1;
    }

    if opts.all && opts.version.is_some() {
        return Err("cannot pass both `--all` and `--version` to `toolchain clean`".to_owned());
    }

    Ok(ParsedSubcommand::Opts(ToolchainCliCommand::Clean(opts)))
}

fn parse_toolchain_package(
    args: &[String],
) -> Result<ParsedSubcommand<ToolchainCliCommand>, String> {
    let mut out: Option<PathBuf> = None;
    let mut mires_root: Option<PathBuf> = None;
    let mut version: Option<String> = None;
    let mut host: Option<String> = None;
    let mut include_cli = false;
    let mut no_sysroots = false;
    let mut verbose = false;
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        if matches!(arg.as_str(), "-h" | "--help") {
            return Ok(ParsedSubcommand::Help);
        }
        let (flag, inline_value) = split_flag(arg);
        match flag {
            "--out" => {
                out = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--mires-root" => {
                mires_root = Some(PathBuf::from(take_value(
                    args,
                    &mut index,
                    flag,
                    inline_value,
                )?));
            }
            "--version" => {
                version = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "--host" => {
                host = Some(take_value(args, &mut index, flag, inline_value)?);
            }
            "--include-cli" => {
                reject_value(flag, inline_value)?;
                include_cli = true;
            }
            "--no-sysroots" => {
                reject_value(flag, inline_value)?;
                no_sysroots = true;
            }
            "-v" | "--verbose" => {
                reject_value(flag, inline_value)?;
                verbose = true;
            }
            other => {
                return Err(format!("unknown option `{other}` for `toolchain package`"));
            }
        }
        index += 1;
    }

    let out =
        out.ok_or_else(|| "`toolchain package` requires `--out <archive.tar.gz>`".to_owned())?;

    Ok(ParsedSubcommand::Opts(ToolchainCliCommand::Package(
        ToolchainPackageOptions {
            out,
            mires_root,
            version,
            host,
            include_cli,
            no_sysroots,
            verbose,
        },
    )))
}

fn split_flag(arg: &str) -> (&str, Option<&str>) {
    if let Some((flag, value)) = arg.split_once('=') {
        if flag.starts_with('-') {
            return (flag, Some(value));
        }
    }
    (arg, None)
}

fn take_value(
    args: &[String],
    index: &mut usize,
    flag: &str,
    inline_value: Option<&str>,
) -> Result<String, String> {
    if let Some(value) = inline_value {
        if value.is_empty() {
            return Err(format!("option `{flag}` requires a non-empty value"));
        }
        return Ok(value.to_owned());
    }
    let next = *index + 1;
    let Some(value) = args.get(next) else {
        return Err(format!("option `{flag}` requires a value"));
    };
    *index = next;
    Ok(value.clone())
}

fn reject_value(flag: &str, inline_value: Option<&str>) -> Result<(), String> {
    if inline_value.is_some() {
        return Err(format!("flag `{flag}` does not take a value"));
    }
    Ok(())
}

/// Format top-level or subcommand usage help.
pub fn help_text(topic: Option<&str>) -> &'static str {
    match topic {
        Some("setup") => {
            "\
Download and install a prebuilt Mires compiler toolchain bundle

Usage: cargo mires setup [OPTIONS]

Options:
      --version <VER>         Toolchain version to install (default: current cargo-mires version)
      --from-archive <PATH>   Install from a local `.tar.gz` toolchain archive instead of downloading
      --url <BASE_URL>        Override release download base URL (env: MIRES_RELEASE_URL)
      --sha256 <HEX>          Verify archive against an explicit SHA-256 hex digest
      --no-rustup             Skip automatic `rustup toolchain install` check for pinned nightly
      --force                 Reinstall even if this version is already cached
  -v, --verbose               Print verbose download and setup logs
  -h, --help                  Print help
"
        }
        Some("toolchain") => {
            "\
Inspect, clean, or package Mires compiler toolchain bundles

Usage: cargo mires toolchain <SUBCOMMAND> [OPTIONS]

Subcommands:
  list      List installed managed Mires toolchains in MIRES_HOME
  path      Print the active resolved Mires toolchain root directory
            Options: [--mires-root <PATH>]
  clean     Remove temporary files or installed toolchains
            Options: [--all | --version <VER>]
  package   Package a Mires source checkout into a release `.tar.gz` bundle + `.sha256`
            Options: --out <ARCHIVE.tar.gz> [--mires-root <DIR>] [--version <VER>]
                     [--host <TRIPLE>] [--include-cli] [--no-sysroots] [--verbose]
"
        }
        Some("sysroot") => {
            "\
Build or refresh the cached Mires sysroot

Usage: cargo mires sysroot [OPTIONS]

Options:
      --clean               Delete and rebuild the matching sysroot cache entry
      --release             Target the `release-balanced` profile
      --profile <PROFILE>   Profile: dev|report|release-size|release-balanced|release-speed
      --panic <MODE>        Panic strategy: unwind|abort (default: unwind)
      --mires-root <PATH>   Path to the Mires toolchain checkout or installed bundle
      --manifest-path <P>   Path to Cargo.toml
  -p, --package <NAME>      Package whose `[package.metadata.mires]` to read
  -v, --verbose             Print verbose command logs
  -h, --help                Print help
"
        }
        Some("build" | "check" | "watch") => {
            "\
Compile, typecheck, or watch a Cargo crate targeting JavaScript via Mires

Usage: cargo mires <build|check|watch> [OPTIONS]

Options:
  -p, --package <NAME>        Package to compile
      --manifest-path <PATH>  Path to Cargo.toml
      --release               Build with release optimizations (default profile: release-balanced)
      --profile <PROFILE>     Profile: dev|report|release-size|release-balanced|release-speed
      --format <FORMAT>       Output module format: esm|script (default: esm)
      --runtime <MODE>        Runtime shim mode: inline|external|none (default: inline)
      --out-dir <DIR>         Output directory (default: <target_dir>/mires)
      --dts / --no-dts        Emit TypeScript .d.ts declarations (default: on for esm)
      --source-map / --no-source-map
                              Emit source map .map files (default: on)
      --npm / --no-npm        Emit package.json in --out-dir
      --panic <MODE>          Panic strategy: unwind|abort (default: unwind)
      --mires-root <PATH>     Path to the Mires toolchain checkout or installed bundle
  -F, --features <LIST>       Space or comma separated Cargo features
      --all-features          Activate all available features
      --no-default-features   Do not activate the `default` feature
      --target-dir <DIR>      Override Cargo target directory
      --lib                   Build only the package's library target
      --bin <NAME>            Build only the named binary target
  -v, --verbose               Print verbose compiler/wrapper logs
  -h, --help                  Print help
"
        }
        _ => {
            "\
Cargo subcommand for compiling Rust crates to JavaScript with Mires

Usage: cargo mires <COMMAND> [OPTIONS]

Commands:
  build      Compile a crate to JavaScript and publish artifacts
  check      Typecheck a crate against the Mires sysroot without codegen
  watch      Rebuild automatically whenever Rust sources or Cargo.toml change
  sysroot    Ensure or rebuild the cached Mires sysroot
  setup      Download and install a prebuilt Mires toolchain bundle
  toolchain  Inspect, clean, or package Mires toolchain bundles
  help       Print this message or help for a subcommand

Options:
  -h, --help     Print help
  -V, --version  Print version
"
        }
    }
}

pub fn version_text() -> String {
    format!("cargo-mires {}", env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_cargo_subcommand_mires_prefix() {
        let direct = parse_args(["cargo-mires", "build", "--release", "--format", "esm"]).unwrap();
        let via_cargo =
            parse_args(["cargo-mires", "mires", "build", "--release", "--format=esm"]).unwrap();
        assert_eq!(direct, via_cargo);

        let Command::Build(opts) = via_cargo else {
            panic!("expected Build command");
        };
        assert!(opts.release);
        assert_eq!(opts.format, Some(OutputFormat::Esm));
    }

    #[test]
    fn parses_all_build_flags() {
        let cmd = parse_args([
            "cargo-mires",
            "mires",
            "build",
            "-p",
            "demo-app",
            "--manifest-path",
            "crates/demo/Cargo.toml",
            "--release",
            "--profile",
            "release-size",
            "--format",
            "script",
            "--runtime=external",
            "--out-dir",
            "dist/js",
            "--no-dts",
            "--no-source-map",
            "--npm",
            "--panic",
            "abort",
            "--mires-root=/opt/mires",
            "--features",
            "a,b",
            "--all-features",
            "--no-default-features",
            "--target-dir",
            "custom-target",
            "--bin",
            "main-app",
            "--verbose",
        ])
        .unwrap();

        let Command::Build(opts) = cmd else {
            panic!("expected Build");
        };
        assert_eq!(opts.package.as_deref(), Some("demo-app"));
        assert_eq!(
            opts.manifest_path,
            Some(PathBuf::from("crates/demo/Cargo.toml"))
        );
        assert!(opts.release);
        assert_eq!(opts.profile, Some(Profile::ReleaseSize));
        assert_eq!(opts.format, Some(OutputFormat::Script));
        assert_eq!(opts.runtime, Some(RuntimeMode::External));
        assert_eq!(opts.out_dir, Some(PathBuf::from("dist/js")));
        assert_eq!(opts.dts, Some(false));
        assert_eq!(opts.source_map, Some(false));
        assert_eq!(opts.npm, Some(true));
        assert_eq!(opts.panic, Some(PanicMode::Abort));
        assert_eq!(opts.mires_root, Some(PathBuf::from("/opt/mires")));
        assert_eq!(opts.features, vec!["a,b".to_owned()]);
        assert!(opts.all_features);
        assert!(opts.no_default_features);
        assert_eq!(opts.target_dir, Some(PathBuf::from("custom-target")));
        assert_eq!(
            opts.target_selection,
            TargetSelection::Bin("main-app".to_owned())
        );
        assert!(opts.verbose);
    }

    #[test]
    fn parses_check_watch_sysroot_help_version() {
        assert!(matches!(
            parse_args(["cargo-mires", "mires", "check", "--lib"]).unwrap(),
            Command::Check(BuildCliOptions {
                target_selection: TargetSelection::Lib,
                ..
            })
        ));
        assert!(matches!(
            parse_args(["cargo-mires", "watch", "--dts", "--source-map"]).unwrap(),
            Command::Watch(BuildCliOptions {
                dts: Some(true),
                source_map: Some(true),
                ..
            })
        ));
        assert_eq!(
            parse_args([
                "cargo-mires",
                "mires",
                "sysroot",
                "--clean",
                "--panic=abort"
            ])
            .unwrap(),
            Command::Sysroot(SysrootCliOptions {
                clean: true,
                panic: Some(PanicMode::Abort),
                ..Default::default()
            })
        );
        assert_eq!(
            parse_args(["cargo-mires", "mires", "--version"]).unwrap(),
            Command::Version
        );
        assert_eq!(
            parse_args(["cargo-mires", "mires", "--help"]).unwrap(),
            Command::Help(None)
        );
        assert_eq!(
            parse_args(["cargo-mires", "build", "--help"]).unwrap(),
            Command::Help(Some("build".to_owned()))
        );
    }

    #[test]
    fn parses_setup_and_toolchain_subcommands() {
        assert_eq!(
            parse_args([
                "cargo-mires",
                "mires",
                "setup",
                "--version",
                "0.2.0",
                "--from-archive=/tmp/tc.tar.gz",
                "--url",
                "https://example.invalid/releases",
                "--sha256",
                "deadbeef",
                "--no-rustup",
                "--force",
                "--verbose",
            ])
            .unwrap(),
            Command::Setup(SetupCliOptions {
                version: Some("0.2.0".to_owned()),
                from_archive: Some(PathBuf::from("/tmp/tc.tar.gz")),
                url: Some("https://example.invalid/releases".to_owned()),
                sha256: Some("deadbeef".to_owned()),
                no_rustup: true,
                force: true,
                verbose: true,
            })
        );

        assert_eq!(
            parse_args(["cargo-mires", "mires", "toolchain", "list"]).unwrap(),
            Command::Toolchain(ToolchainCliCommand::List)
        );
        assert_eq!(
            parse_args([
                "cargo-mires",
                "toolchain",
                "path",
                "--mires-root",
                "/opt/mires"
            ])
            .unwrap(),
            Command::Toolchain(ToolchainCliCommand::Path(ToolchainPathOptions {
                mires_root: Some(PathBuf::from("/opt/mires")),
            }))
        );
        assert_eq!(
            parse_args(["cargo-mires", "toolchain", "clean", "--all"]).unwrap(),
            Command::Toolchain(ToolchainCliCommand::Clean(ToolchainCleanOptions {
                all: true,
                version: None,
            }))
        );
        assert_eq!(
            parse_args(["cargo-mires", "toolchain", "clean", "--version=0.1.0"]).unwrap(),
            Command::Toolchain(ToolchainCliCommand::Clean(ToolchainCleanOptions {
                all: false,
                version: Some("0.1.0".to_owned()),
            }))
        );
        assert_eq!(
            parse_args([
                "cargo-mires",
                "toolchain",
                "package",
                "--out",
                "dist/mires.tar.gz",
                "--mires-root=/src/mires",
                "--version",
                "0.1.0",
                "--host",
                "aarch64-apple-darwin",
                "--include-cli",
                "--no-sysroots",
                "-v",
            ])
            .unwrap(),
            Command::Toolchain(ToolchainCliCommand::Package(ToolchainPackageOptions {
                out: PathBuf::from("dist/mires.tar.gz"),
                mires_root: Some(PathBuf::from("/src/mires")),
                version: Some("0.1.0".to_owned()),
                host: Some("aarch64-apple-darwin".to_owned()),
                include_cli: true,
                no_sysroots: true,
                verbose: true,
            }))
        );
        assert_eq!(
            parse_args(["cargo-mires", "setup", "--help"]).unwrap(),
            Command::Help(Some("setup".to_owned()))
        );
        assert_eq!(
            parse_args(["cargo-mires", "toolchain", "--help"]).unwrap(),
            Command::Help(Some("toolchain".to_owned()))
        );
    }

    #[test]
    fn rejects_invalid_and_conflicting_flags() {
        assert!(parse_args(["cargo-mires", "build", "--lib", "--bin", "foo"]).is_err());
        assert!(parse_args(["cargo-mires", "build", "--profile", "bogus"]).is_err());
        assert!(parse_args(["cargo-mires", "build", "--format", "cjs"]).is_err());
        assert!(parse_args(["cargo-mires", "build", "--runtime", "weird"]).is_err());
        assert!(parse_args(["cargo-mires", "build", "--panic", "trap"]).is_err());
        assert!(parse_args(["cargo-mires", "build", "--out-dir"]).is_err());
        assert!(parse_args(["cargo-mires", "build", "--release=true"]).is_err());
        assert!(parse_args([
            "cargo-mires",
            "toolchain",
            "clean",
            "--all",
            "--version",
            "0.1.0"
        ])
        .is_err());
        assert!(parse_args(["cargo-mires", "toolchain", "package"]).is_err());
        assert!(parse_args(["cargo-mires", "toolchain", "bogus"]).is_err());
        assert!(parse_args(["cargo-mires", "bogus"]).is_err());
    }
}
