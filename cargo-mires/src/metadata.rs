//! Cargo metadata discovery and `[package.metadata.mires]` resolution.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::cli::{BuildCliOptions, OutputFormat, PanicMode, Profile, RuntimeMode, TargetSelection};

/// Selected Cargo target to build or check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectedTarget {
    Lib { name: String },
    Bin { name: String },
}

impl SelectedTarget {
    pub fn name(&self) -> &str {
        match self {
            Self::Lib { name } | Self::Bin { name } => name,
        }
    }
}

/// Package information extracted from `cargo metadata`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub license: Option<String>,
    pub edition: String,
    pub manifest_path: PathBuf,
    pub manifest_dir: PathBuf,
}

/// Fully merged build configuration after applying `CLI > [package.metadata.mires] > defaults`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConfig {
    pub profile: Profile,
    pub cargo_release: bool,
    pub format: OutputFormat,
    pub runtime: RuntimeMode,
    pub dts: bool,
    pub source_map: bool,
    pub npm: bool,
    pub panic: PanicMode,
    pub out_dir: PathBuf,
    pub shim_module: String,
    pub chunks: Vec<(String, String)>,
    pub shared_chunk: Option<String>,
}

/// Complete workspace + package + merged configuration context for a command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageContext {
    pub workspace_root: PathBuf,
    pub target_directory: PathBuf,
    pub package: PackageInfo,
    pub target: SelectedTarget,
    pub config: ResolvedConfig,
}

/// Raw `[package.metadata.mires]` table deserialized from `cargo metadata` JSON.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct MiresMetadata {
    pub format: Option<String>,
    pub runtime: Option<String>,
    pub dts: Option<bool>,
    pub source_map: Option<bool>,
    pub profile: Option<String>,
    pub out_dir: Option<String>,
    pub npm: Option<bool>,
    pub shim_module: Option<String>,
    pub panic: Option<String>,
    pub chunks: Option<ChunksMetadata>,
    pub islands: Option<Vec<String>>,
    pub shared_chunk: Option<String>,
}

/// Flexible chunk specification in `[package.metadata.mires]`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
pub enum ChunksMetadata {
    Map(BTreeMap<String, String>),
    List(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    #[serde(default)]
    workspace_default_members: Vec<String>,
    target_directory: PathBuf,
    workspace_root: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    version: String,
    description: Option<String>,
    license: Option<String>,
    edition: String,
    manifest_path: PathBuf,
    targets: Vec<CargoTarget>,
    #[serde(default)]
    metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct CargoTarget {
    name: String,
    kind: Vec<String>,
    crate_types: Vec<String>,
}

/// Run `cargo metadata --format-version=1 --no-deps` and resolve package + settings.
pub fn load_package_context(opts: &BuildCliOptions) -> Result<PackageContext, String> {
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--format-version=1", "--no-deps"]);
    if let Some(manifest_path) = &opts.manifest_path {
        cmd.arg("--manifest-path").arg(manifest_path);
    }
    let output = cmd
        .output()
        .map_err(|err| format!("failed to run `cargo metadata`: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("`cargo metadata` exited with {}", output.status)
        } else {
            format!("`cargo metadata` failed:\n{stderr}")
        });
    }

    let cwd = std::env::current_dir().ok();
    parse_metadata_json(&output.stdout, opts, cwd.as_deref())
}

/// Optional metadata lookup used by `cargo mires sysroot` when invoked inside a crate.
pub fn try_load_sysroot_metadata(
    manifest_path: Option<&Path>,
    package: Option<&str>,
) -> Option<MiresMetadata> {
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--format-version=1", "--no-deps"]);
    if let Some(path) = manifest_path {
        cmd.arg("--manifest-path").arg(path);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let meta: CargoMetadata = serde_json::from_slice(&output.stdout).ok()?;
    let cwd = std::env::current_dir().ok();
    let pkg = select_package(&meta, package, manifest_path, cwd.as_deref()).ok()?;
    parse_mires_metadata(&pkg.metadata).ok()
}

fn parse_metadata_json(
    bytes: &[u8],
    opts: &BuildCliOptions,
    cwd: Option<&Path>,
) -> Result<PackageContext, String> {
    let meta: CargoMetadata = serde_json::from_slice(bytes)
        .map_err(|err| format!("failed to parse `cargo metadata` JSON: {err}"))?;

    let pkg = select_package(
        &meta,
        opts.package.as_deref(),
        opts.manifest_path.as_deref(),
        cwd,
    )?;

    let target = select_target(pkg, &opts.target_selection)?;
    let mires_meta = parse_mires_metadata(&pkg.metadata)?;

    let manifest_dir = pkg
        .manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let effective_target_dir = opts
        .target_dir
        .clone()
        .unwrap_or_else(|| meta.target_directory.clone());

    let config = merge_config(opts, &mires_meta, &manifest_dir, &effective_target_dir)?;

    let package = PackageInfo {
        id: pkg.id.clone(),
        name: pkg.name.clone(),
        version: pkg.version.clone(),
        description: pkg.description.clone(),
        license: pkg.license.clone(),
        edition: pkg.edition.clone(),
        manifest_path: pkg.manifest_path.clone(),
        manifest_dir,
    };

    Ok(PackageContext {
        workspace_root: meta.workspace_root,
        target_directory: effective_target_dir,
        package,
        target,
        config,
    })
}

fn select_package<'a>(
    meta: &'a CargoMetadata,
    package_name: Option<&str>,
    manifest_path: Option<&Path>,
    cwd: Option<&Path>,
) -> Result<&'a CargoPackage, String> {
    if let Some(name) = package_name {
        return meta
            .packages
            .iter()
            .find(|pkg| pkg.name == name)
            .ok_or_else(|| {
                let names: Vec<&str> = meta.packages.iter().map(|p| p.name.as_str()).collect();
                format!(
                    "package `{name}` not found in workspace (available: {})",
                    names.join(", ")
                )
            });
    }

    if let Some(manifest) = manifest_path {
        let wanted = canonical_or_owned(manifest);
        if let Some(pkg) = meta
            .packages
            .iter()
            .find(|pkg| canonical_or_owned(&pkg.manifest_path) == wanted)
        {
            return Ok(pkg);
        }
    }

    if let Some(cwd) = cwd {
        let cwd_norm = canonical_or_owned(cwd);
        let mut best: Option<(&CargoPackage, usize)> = None;
        for pkg in &meta.packages {
            let Some(dir) = pkg.manifest_path.parent() else {
                continue;
            };
            let dir_norm = canonical_or_owned(dir);
            if cwd_norm.starts_with(&dir_norm) {
                let depth = dir_norm.components().count();
                if best.is_none_or(|(_, best_depth)| depth > best_depth) {
                    best = Some((pkg, depth));
                }
            }
        }
        if let Some((pkg, _)) = best {
            return Ok(pkg);
        }
    }

    if meta.workspace_default_members.len() == 1 {
        let default_id = &meta.workspace_default_members[0];
        if let Some(pkg) = meta.packages.iter().find(|pkg| &pkg.id == default_id) {
            return Ok(pkg);
        }
    }

    if meta.packages.len() == 1 {
        return Ok(&meta.packages[0]);
    }

    let names: Vec<&str> = meta.packages.iter().map(|p| p.name.as_str()).collect();
    Err(format!(
        "workspace contains multiple packages; pass `-p <name>` (available: {})",
        names.join(", ")
    ))
}

fn select_target(
    pkg: &CargoPackage,
    selection: &TargetSelection,
) -> Result<SelectedTarget, String> {
    let lib_target = pkg.targets.iter().find(|t| is_lib_target(t));
    let bin_targets: Vec<&CargoTarget> = pkg.targets.iter().filter(|t| is_bin_target(t)).collect();

    match selection {
        TargetSelection::Lib => {
            let target = lib_target
                .ok_or_else(|| format!("package `{}` has no library target", pkg.name))?;
            Ok(SelectedTarget::Lib {
                name: target.name.clone(),
            })
        }
        TargetSelection::Bin(bin_name) => {
            let target = bin_targets
                .iter()
                .find(|t| t.name == *bin_name)
                .ok_or_else(|| {
                    format!("package `{}` has no binary target `{bin_name}`", pkg.name)
                })?;
            Ok(SelectedTarget::Bin {
                name: target.name.clone(),
            })
        }
        TargetSelection::Auto => {
            if let Some(lib) = lib_target {
                if lib.crate_types.iter().any(|ct| ct == "cdylib") {
                    return Ok(SelectedTarget::Lib {
                        name: lib.name.clone(),
                    });
                }
            }
            if bin_targets.len() == 1 {
                return Ok(SelectedTarget::Bin {
                    name: bin_targets[0].name.clone(),
                });
            }
            if let Some(lib) = lib_target {
                if bin_targets.is_empty() {
                    return Ok(SelectedTarget::Lib {
                        name: lib.name.clone(),
                    });
                }
            }
            if bin_targets.len() > 1 {
                let names: Vec<&str> = bin_targets.iter().map(|b| b.name.as_str()).collect();
                return Err(format!(
                    "package `{}` has multiple binary targets ({}); pass `--bin <name>` or `--lib`",
                    pkg.name,
                    names.join(", ")
                ));
            }
            Err(format!(
                "package `{}` has no library or binary target",
                pkg.name
            ))
        }
    }
}

fn is_lib_target(target: &CargoTarget) -> bool {
    target
        .kind
        .iter()
        .any(|k| matches!(k.as_str(), "lib" | "rlib" | "cdylib" | "dylib"))
}

fn is_bin_target(target: &CargoTarget) -> bool {
    target.kind.iter().any(|k| k == "bin")
}

/// Extract `[package.metadata.mires]` from the package's `metadata` JSON value.
pub fn parse_mires_metadata(metadata: &serde_json::Value) -> Result<MiresMetadata, String> {
    let Some(mires_val) = metadata.get("mires") else {
        return Ok(MiresMetadata::default());
    };
    if mires_val.is_null() {
        return Ok(MiresMetadata::default());
    }
    serde_json::from_value(mires_val.clone())
        .map_err(|err| format!("invalid `[package.metadata.mires]`: {err}"))
}

/// Merge CLI options, `[package.metadata.mires]`, and defaults with strict precedence:
/// CLI > metadata > defaults.
pub fn merge_config(
    cli: &BuildCliOptions,
    meta: &MiresMetadata,
    manifest_dir: &Path,
    target_dir: &Path,
) -> Result<ResolvedConfig, String> {
    let meta_format = parse_opt_field(
        "format",
        meta.format.as_deref(),
        OutputFormat::parse,
        "esm` or `script",
    )?;
    let meta_runtime = parse_opt_field(
        "runtime",
        meta.runtime.as_deref(),
        RuntimeMode::parse,
        "inline`, `external`, or `none",
    )?;
    let meta_profile = parse_opt_field(
        "profile",
        meta.profile.as_deref(),
        Profile::parse,
        "dev`, `report`, `release-size`, `release-balanced`, or `release-speed",
    )?;
    let meta_panic = parse_opt_field(
        "panic",
        meta.panic.as_deref(),
        PanicMode::parse,
        "unwind` or `abort",
    )?;

    let chunks = collect_chunks(meta)?;
    let has_chunking = !chunks.is_empty();

    let format = cli.format.or(meta_format).unwrap_or(OutputFormat::Esm);
    let runtime = cli.runtime.or(meta_runtime).unwrap_or(if has_chunking {
        RuntimeMode::External
    } else {
        RuntimeMode::Inline
    });
    let dts = cli.dts.or(meta.dts).unwrap_or(format == OutputFormat::Esm);
    let source_map = cli.source_map.or(meta.source_map).unwrap_or(true);
    let npm = cli.npm.or(meta.npm).unwrap_or(false);
    let panic = cli.panic.or(meta_panic).unwrap_or(PanicMode::Unwind);

    let profile = if let Some(explicit) = cli.profile {
        explicit
    } else if cli.release {
        match meta_profile {
            Some(p) if p.is_release() => p,
            _ => Profile::ReleaseBalanced,
        }
    } else {
        meta_profile.unwrap_or(Profile::Dev)
    };

    let cargo_release = cli.release || profile.is_release();

    let out_dir = if let Some(dir) = &cli.out_dir {
        dir.clone()
    } else if let Some(dir) = &meta.out_dir {
        let path = PathBuf::from(dir);
        if path.is_absolute() {
            path
        } else {
            manifest_dir.join(path)
        }
    } else {
        target_dir.join("mires")
    };

    let shim_module = meta.shim_module.clone().unwrap_or_else(|| match format {
        OutputFormat::Esm => "./shim.mjs".to_owned(),
        OutputFormat::Script => "./shim.js".to_owned(),
    });

    Ok(ResolvedConfig {
        profile,
        cargo_release,
        format,
        runtime,
        dts,
        source_map,
        npm,
        panic,
        out_dir,
        shim_module,
        chunks,
        shared_chunk: meta.shared_chunk.clone(),
    })
}

fn collect_chunks(meta: &MiresMetadata) -> Result<Vec<(String, String)>, String> {
    let mut chunks = Vec::new();
    if let Some(spec) = &meta.chunks {
        match spec {
            ChunksMetadata::Map(map) => {
                for (name, entry) in map {
                    if name.is_empty() || entry.is_empty() {
                        return Err(
                            "invalid `[package.metadata.mires].chunks` entry: empty name or entry"
                                .to_owned(),
                        );
                    }
                    chunks.push((name.clone(), entry.clone()));
                }
            }
            ChunksMetadata::List(list) => {
                for item in list {
                    let (name, entry) = item.split_once(':').ok_or_else(|| {
                        format!(
                            "invalid `[package.metadata.mires].chunks` entry `{item}` \
                             (expected `<name>:<entry>`)"
                        )
                    })?;
                    if name.is_empty() || entry.is_empty() {
                        return Err(format!(
                            "invalid `[package.metadata.mires].chunks` entry `{item}`"
                        ));
                    }
                    chunks.push((name.to_owned(), entry.to_owned()));
                }
            }
        }
    }
    if let Some(islands) = &meta.islands {
        for item in islands {
            let (name, entry) = match item.split_once(':') {
                Some((n, e)) => (n, e),
                None => (item.as_str(), item.as_str()),
            };
            if name.is_empty() || entry.is_empty() {
                return Err(format!(
                    "invalid `[package.metadata.mires].islands` entry `{item}`"
                ));
            }
            chunks.push((name.to_owned(), entry.to_owned()));
        }
    }
    Ok(chunks)
}

fn parse_opt_field<T>(
    field: &str,
    raw: Option<&str>,
    parser: fn(&str) -> Option<T>,
    expected: &str,
) -> Result<Option<T>, String> {
    let Some(value) = raw else {
        return Ok(None);
    };
    parser(value).map(Some).ok_or_else(|| {
        format!(
            "invalid `[package.metadata.mires].{field}` value `{value}` (expected `{expected}`)"
        )
    })
}

fn canonical_or_owned(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_specification() {
        let cli = BuildCliOptions::default();
        let meta = MiresMetadata::default();
        let cfg = merge_config(
            &cli,
            &meta,
            Path::new("/ws/crate-a"),
            Path::new("/ws/target"),
        )
        .unwrap();

        assert_eq!(cfg.profile, Profile::Dev);
        assert!(!cfg.cargo_release);
        assert_eq!(cfg.format, OutputFormat::Esm);
        assert_eq!(cfg.runtime, RuntimeMode::Inline);
        assert!(cfg.dts);
        assert!(cfg.source_map);
        assert!(!cfg.npm);
        assert_eq!(cfg.panic, PanicMode::Unwind);
        assert_eq!(cfg.out_dir, PathBuf::from("/ws/target/mires"));
        assert_eq!(cfg.shim_module, "./shim.mjs");
    }

    #[test]
    fn metadata_overrides_defaults_and_cli_overrides_metadata() {
        let meta = MiresMetadata {
            format: Some("script".to_owned()),
            runtime: Some("external".to_owned()),
            dts: Some(true),
            source_map: Some(false),
            profile: Some("release-size".to_owned()),
            out_dir: Some("dist".to_owned()),
            npm: Some(true),
            shim_module: Some("./runtime/rt.mjs".to_owned()),
            panic: Some("abort".to_owned()),
            chunks: None,
            islands: None,
            shared_chunk: None,
        };

        // Metadata beats defaults
        let cli_empty = BuildCliOptions::default();
        let cfg = merge_config(
            &cli_empty,
            &meta,
            Path::new("/ws/my-pkg"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert_eq!(cfg.format, OutputFormat::Script);
        assert_eq!(cfg.runtime, RuntimeMode::External);
        assert!(cfg.dts);
        assert!(!cfg.source_map);
        assert_eq!(cfg.profile, Profile::ReleaseSize);
        assert!(cfg.cargo_release);
        assert_eq!(cfg.out_dir, PathBuf::from("/ws/my-pkg/dist"));
        assert!(cfg.npm);
        assert_eq!(cfg.shim_module, "./runtime/rt.mjs");
        assert_eq!(cfg.panic, PanicMode::Abort);

        // CLI beats metadata
        let cli_override = BuildCliOptions {
            format: Some(OutputFormat::Esm),
            runtime: Some(RuntimeMode::None),
            dts: Some(false),
            source_map: Some(true),
            profile: Some(Profile::Dev),
            out_dir: Some(PathBuf::from("override-out")),
            npm: Some(false),
            panic: Some(PanicMode::Unwind),
            ..Default::default()
        };
        let cfg2 = merge_config(
            &cli_override,
            &meta,
            Path::new("/ws/my-pkg"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert_eq!(cfg2.format, OutputFormat::Esm);
        assert_eq!(cfg2.runtime, RuntimeMode::None);
        assert!(!cfg2.dts);
        assert!(cfg2.source_map);
        assert_eq!(cfg2.profile, Profile::Dev);
        assert!(!cfg2.cargo_release);
        assert_eq!(cfg2.out_dir, PathBuf::from("override-out"));
        assert!(!cfg2.npm);
        assert_eq!(cfg2.panic, PanicMode::Unwind);
    }

    #[test]
    fn release_flag_and_script_dts_and_chunking_defaults() {
        // --release upgrades default dev to release-balanced
        let cli_release = BuildCliOptions {
            release: true,
            ..Default::default()
        };
        let cfg = merge_config(
            &cli_release,
            &MiresMetadata::default(),
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert_eq!(cfg.profile, Profile::ReleaseBalanced);
        assert!(cfg.cargo_release);

        // --release respects release-speed from metadata
        let meta_speed = MiresMetadata {
            profile: Some("release-speed".to_owned()),
            ..Default::default()
        };
        let cfg_speed = merge_config(
            &cli_release,
            &meta_speed,
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert_eq!(cfg_speed.profile, Profile::ReleaseSpeed);

        // --format script defaults dts to false
        let cli_script = BuildCliOptions {
            format: Some(OutputFormat::Script),
            ..Default::default()
        };
        let cfg_script = merge_config(
            &cli_script,
            &MiresMetadata::default(),
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert!(!cfg_script.dts);

        // islands/chunking defaults runtime to external
        let meta_islands = MiresMetadata {
            islands: Some(vec!["counter".to_owned(), "search:search_init".to_owned()]),
            ..Default::default()
        };
        let cfg_islands = merge_config(
            &BuildCliOptions::default(),
            &meta_islands,
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert_eq!(cfg_islands.runtime, RuntimeMode::External);
        assert_eq!(
            cfg_islands.chunks,
            vec![
                ("counter".to_owned(), "counter".to_owned()),
                ("search".to_owned(), "search_init".to_owned()),
            ]
        );
    }

    #[test]
    fn parses_cargo_metadata_json_and_selects_targets() {
        let json = serde_json::json!({
            "packages": [
                {
                    "id": "demo-lib 0.2.0 (path+file:///ws/demo-lib)",
                    "name": "demo-lib",
                    "version": "0.2.0",
                    "description": "Demo library",
                    "license": "MIT",
                    "edition": "2021",
                    "manifest_path": "/ws/demo-lib/Cargo.toml",
                    "targets": [
                        {
                            "name": "demo_lib",
                            "kind": ["cdylib", "rlib"],
                            "crate_types": ["cdylib", "rlib"]
                        }
                    ],
                    "metadata": {
                        "mires": {
                            "format": "esm",
                            "runtime": "external",
                            "out-dir": "dist",
                            "shim-module": "./rt.mjs",
                            "npm": true
                        }
                    }
                }
            ],
            "workspace_default_members": ["demo-lib 0.2.0 (path+file:///ws/demo-lib)"],
            "target_directory": "/ws/target",
            "workspace_root": "/ws"
        });

        let ctx = parse_metadata_json(
            json.to_string().as_bytes(),
            &BuildCliOptions::default(),
            Some(Path::new("/ws")),
        )
        .unwrap();

        assert_eq!(ctx.package.name, "demo-lib");
        assert_eq!(ctx.package.version, "0.2.0");
        assert_eq!(ctx.package.description.as_deref(), Some("Demo library"));
        assert_eq!(ctx.package.license.as_deref(), Some("MIT"));
        assert_eq!(
            ctx.target,
            SelectedTarget::Lib {
                name: "demo_lib".to_owned()
            }
        );
        assert_eq!(ctx.config.runtime, RuntimeMode::External);
        assert_eq!(ctx.config.out_dir, PathBuf::from("/ws/demo-lib/dist"));
        assert_eq!(ctx.config.shim_module, "./rt.mjs");
        assert!(ctx.config.npm);
    }

    #[test]
    fn rejects_invalid_metadata_values() {
        let bad = MiresMetadata {
            format: Some("amd".to_owned()),
            ..Default::default()
        };
        assert!(merge_config(
            &BuildCliOptions::default(),
            &bad,
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .is_err());
    }
}
