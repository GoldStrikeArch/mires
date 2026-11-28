//! Cargo metadata discovery and `[package.metadata.mires]` resolution.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use crate::cli::{
    BuildCliOptions, DiscoverChunks, MessageFormat, OutputFormat, PanicMode, Profile, RuntimeMode,
    TargetSelection,
};

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

/// Explicit chunk request resolved from CLI flags or `[package.metadata.mires]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkRequest {
    pub name: String,
    pub entry: String,
    pub role: Option<String>,
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
    pub chunks: Vec<ChunkRequest>,
    pub shared_chunk: Option<String>,
    pub discover_chunks: DiscoverChunks,
    pub cfgs: Vec<String>,
    pub message_format: MessageFormat,
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
    pub discover_chunks: Option<String>,
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
    let meta_discover_chunks = parse_opt_field(
        "discover-chunks",
        meta.discover_chunks.as_deref(),
        DiscoverChunks::parse,
        "auto`, `on`, or `off",
    )?;

    let chunks = collect_chunks(cli, meta)?;
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
    let discover_chunks = cli
        .discover_chunks
        .or(meta_discover_chunks)
        .unwrap_or(DiscoverChunks::Auto);

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

    let shim_module = cli
        .shim_module
        .clone()
        .or_else(|| meta.shim_module.clone())
        .unwrap_or_else(|| match format {
            OutputFormat::Esm => "./shim.mjs".to_owned(),
            OutputFormat::Script => "./shim.js".to_owned(),
        });

    let shared_chunk = cli
        .shared_chunk
        .clone()
        .or_else(|| meta.shared_chunk.clone());

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
        shared_chunk,
        discover_chunks,
        cfgs: cli.cfgs.clone(),
        message_format: cli.message_format.unwrap_or(MessageFormat::Human),
    })
}

fn collect_chunks(
    cli: &BuildCliOptions,
    meta: &MiresMetadata,
) -> Result<Vec<ChunkRequest>, String> {
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
                    upsert_chunk(
                        &mut chunks,
                        ChunkRequest {
                            name: name.clone(),
                            entry: entry.clone(),
                            role: None,
                        },
                    );
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
                    upsert_chunk(
                        &mut chunks,
                        ChunkRequest {
                            name: name.to_owned(),
                            entry: entry.to_owned(),
                            role: None,
                        },
                    );
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
            upsert_chunk(
                &mut chunks,
                ChunkRequest {
                    name: name.to_owned(),
                    entry: entry.to_owned(),
                    role: Some("island".to_owned()),
                },
            );
        }
    }
    for item in &cli.chunks {
        let (name, entry) = item.split_once(':').ok_or_else(|| {
            format!("invalid `--chunk` entry `{item}` (expected `<name>:<entry>`)")
        })?;
        if name.is_empty() || entry.is_empty() {
            return Err(format!("invalid `--chunk` entry `{item}`"));
        }
        upsert_chunk(
            &mut chunks,
            ChunkRequest {
                name: name.to_owned(),
                entry: entry.to_owned(),
                role: Some("main".to_owned()),
            },
        );
    }
    for item in &cli.islands {
        let (name, entry) = match item.split_once(':') {
            Some((n, e)) => (n.to_owned(), e.to_owned()),
            None => (item.clone(), format!("__island_{item}")),
        };
        if name.is_empty() || entry.is_empty() {
            return Err(format!("invalid `--island` entry `{item}`"));
        }
        upsert_chunk(
            &mut chunks,
            ChunkRequest {
                name,
                entry,
                role: Some("island".to_owned()),
            },
        );
    }
    Ok(chunks)
}

fn upsert_chunk(chunks: &mut Vec<ChunkRequest>, req: ChunkRequest) {
    if let Some(existing) = chunks.iter_mut().find(|c| c.name == req.name) {
        *existing = req;
    } else {
        chunks.push(req);
    }
}

#[derive(Debug, Deserialize)]
struct CargoDepMetadata {
    packages: Vec<CargoDepPackage>,
    resolve: Option<CargoDepResolve>,
    workspace_root: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CargoDepPackage {
    id: String,
    source: Option<String>,
    manifest_path: PathBuf,
    #[serde(default)]
    targets: Vec<CargoDepTarget>,
}

#[derive(Debug, Deserialize)]
struct CargoDepTarget {
    #[serde(default)]
    src_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CargoDepResolve {
    #[serde(default)]
    nodes: Vec<CargoDepNode>,
}

#[derive(Debug, Deserialize)]
struct CargoDepNode {
    id: String,
    #[serde(default)]
    dependencies: Vec<String>,
}

/// Collect manifests, source directories, and `.rs` files for the target crate and every
/// transitive local path/workspace dependency so downstream build scripts (`mires-build`)
/// emit complete `cargo:rerun-if-changed` directives.
pub fn collect_dep_files(ctx: &PackageContext, opts: &BuildCliOptions) -> Vec<PathBuf> {
    let mut cmd = Command::new("cargo");
    cmd.args(["metadata", "--format-version=1"]);
    cmd.arg("--manifest-path").arg(&ctx.package.manifest_path);
    if opts.no_default_features {
        cmd.arg("--no-default-features");
    }
    if opts.all_features {
        cmd.arg("--all-features");
    }
    for feat in &opts.features {
        cmd.arg("--features").arg(feat);
    }

    if let Ok(output) = cmd.output() {
        if output.status.success() {
            if let Ok(files) = collect_dep_files_from_metadata_json(&output.stdout, &ctx.package.id)
            {
                if !files.is_empty() {
                    return files;
                }
            }
        }
    }

    let mut fallback = BTreeSet::new();
    collect_local_package_paths(&ctx.package.manifest_path, &[], &mut fallback);
    collect_workspace_root_files(&ctx.workspace_root, &mut fallback);
    fallback.into_iter().collect()
}

/// Parse `cargo metadata --format-version=1` JSON and collect watched files/directories for
/// `root_pkg_id` and every transitive local path dependency (`source == null`).
pub fn collect_dep_files_from_metadata_json(
    bytes: &[u8],
    root_pkg_id: &str,
) -> Result<Vec<PathBuf>, String> {
    let meta: CargoDepMetadata = serde_json::from_slice(bytes)
        .map_err(|err| format!("failed to parse `cargo metadata` dependency graph: {err}"))?;

    let pkg_by_id: BTreeMap<&str, &CargoDepPackage> = meta
        .packages
        .iter()
        .map(|pkg| (pkg.id.as_str(), pkg))
        .collect();
    let deps_by_id: BTreeMap<&str, &[String]> = meta
        .resolve
        .as_ref()
        .map(|resolve| {
            resolve
                .nodes
                .iter()
                .map(|node| (node.id.as_str(), node.dependencies.as_slice()))
                .collect()
        })
        .unwrap_or_default();

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(root_pkg_id);
    visited.insert(root_pkg_id);

    let mut watched = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        if let Some(pkg) = pkg_by_id.get(id) {
            if pkg.source.is_none() {
                let target_srcs: Vec<&Path> = pkg
                    .targets
                    .iter()
                    .filter_map(|t| t.src_path.as_deref())
                    .collect();
                collect_local_package_paths(&pkg.manifest_path, &target_srcs, &mut watched);
            }
        }
        if let Some(deps) = deps_by_id.get(id) {
            for dep in *deps {
                if visited.insert(dep.as_str()) {
                    queue.push_back(dep.as_str());
                }
            }
        }
    }

    collect_workspace_root_files(&meta.workspace_root, &mut watched);
    Ok(watched.into_iter().collect())
}

fn collect_local_package_paths(
    manifest_path: &Path,
    target_srcs: &[&Path],
    out: &mut BTreeSet<PathBuf>,
) {
    out.insert(manifest_path.to_path_buf());
    if let Some(pkg_dir) = manifest_path.parent() {
        let src_dir = pkg_dir.join("src");
        if src_dir.is_dir() {
            collect_source_tree(&src_dir, out);
        }
        let build_rs = pkg_dir.join("build.rs");
        if build_rs.is_file() {
            out.insert(build_rs);
        }
    }
    for src in target_srcs {
        if src.is_file() {
            out.insert((*src).to_path_buf());
        }
    }
}

fn collect_source_tree(dir: &Path, out: &mut BTreeSet<PathBuf>) {
    out.insert(dir.to_path_buf());
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if name != "target" && !name.starts_with('.') {
                collect_source_tree(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.insert(path);
        }
    }
}

fn collect_workspace_root_files(workspace_root: &Path, out: &mut BTreeSet<PathBuf>) {
    let ws_manifest = workspace_root.join("Cargo.toml");
    if ws_manifest.is_file() {
        out.insert(ws_manifest);
    }
    let ws_lock = workspace_root.join("Cargo.lock");
    if ws_lock.is_file() {
        out.insert(ws_lock);
    }
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
            discover_chunks: Some("off".to_owned()),
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
        assert_eq!(cfg.discover_chunks, DiscoverChunks::Off);

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
            discover_chunks: Some(DiscoverChunks::On),
            shim_module: Some("topcoat-dom".to_owned()),
            shared_chunk: Some("common".to_owned()),
            message_format: Some(MessageFormat::Json),
            cfgs: vec!["topcoat_client".to_owned()],
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
        assert_eq!(cfg2.discover_chunks, DiscoverChunks::On);
        assert_eq!(cfg2.shim_module, "topcoat-dom");
        assert_eq!(cfg2.shared_chunk.as_deref(), Some("common"));
        assert_eq!(cfg2.message_format, MessageFormat::Json);
        assert_eq!(cfg2.cfgs, vec!["topcoat_client".to_owned()]);
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

        // islands/chunking defaults runtime to external and assigns island role
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
                ChunkRequest {
                    name: "counter".to_owned(),
                    entry: "counter".to_owned(),
                    role: Some("island".to_owned()),
                },
                ChunkRequest {
                    name: "search".to_owned(),
                    entry: "search_init".to_owned(),
                    role: Some("island".to_owned()),
                },
            ]
        );

        // CLI --island defaults entry to `__island_<NAME>` and overrides metadata
        let cli_chunks = BuildCliOptions {
            chunks: vec!["app:boot".to_owned()],
            islands: vec!["counter".to_owned(), "modal:open_modal".to_owned()],
            ..Default::default()
        };
        let cfg_cli_chunks = merge_config(
            &cli_chunks,
            &meta_islands,
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .unwrap();
        assert_eq!(
            cfg_cli_chunks.chunks,
            vec![
                ChunkRequest {
                    name: "counter".to_owned(),
                    entry: "__island_counter".to_owned(),
                    role: Some("island".to_owned()),
                },
                ChunkRequest {
                    name: "search".to_owned(),
                    entry: "search_init".to_owned(),
                    role: Some("island".to_owned()),
                },
                ChunkRequest {
                    name: "app".to_owned(),
                    entry: "boot".to_owned(),
                    role: Some("main".to_owned()),
                },
                ChunkRequest {
                    name: "modal".to_owned(),
                    entry: "open_modal".to_owned(),
                    role: Some("island".to_owned()),
                },
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

        let bad_discover = MiresMetadata {
            discover_chunks: Some("never".to_owned()),
            ..Default::default()
        };
        assert!(merge_config(
            &BuildCliOptions::default(),
            &bad_discover,
            Path::new("/ws"),
            Path::new("/ws/target"),
        )
        .is_err());
    }

    #[test]
    fn collects_dep_files_across_transitive_local_path_dependencies_only() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let ws =
            std::env::temp_dir().join(format!("cargo-mires-deps-{}-{nanos}", std::process::id()));
        let client = ws.join("client");
        let shared = ws.join("shared");
        let unused = ws.join("server");
        fs::create_dir_all(client.join("src")).unwrap();
        fs::create_dir_all(shared.join("src/nested")).unwrap();
        fs::create_dir_all(unused.join("src")).unwrap();

        fs::write(ws.join("Cargo.toml"), "[workspace]\n").unwrap();
        fs::write(ws.join("Cargo.lock"), "# lock\n").unwrap();
        fs::write(client.join("Cargo.toml"), "[package]\nname = \"client\"\n").unwrap();
        fs::write(client.join("src/lib.rs"), "").unwrap();
        fs::write(shared.join("Cargo.toml"), "[package]\nname = \"shared\"\n").unwrap();
        fs::write(shared.join("src/lib.rs"), "").unwrap();
        fs::write(shared.join("src/nested/model.rs"), "").unwrap();
        fs::write(unused.join("Cargo.toml"), "[package]\nname = \"server\"\n").unwrap();
        fs::write(unused.join("src/main.rs"), "").unwrap();

        let json = serde_json::json!({
            "packages": [
                {
                    "id": "client 0.1.0",
                    "source": null,
                    "manifest_path": client.join("Cargo.toml"),
                    "targets": [{ "src_path": client.join("src/lib.rs") }]
                },
                {
                    "id": "shared 0.1.0",
                    "source": null,
                    "manifest_path": shared.join("Cargo.toml"),
                    "targets": [{ "src_path": shared.join("src/lib.rs") }]
                },
                {
                    "id": "server 0.1.0",
                    "source": null,
                    "manifest_path": unused.join("Cargo.toml"),
                    "targets": [{ "src_path": unused.join("src/main.rs") }]
                },
                {
                    "id": "serde 1.0.0",
                    "source": "registry+https://github.com/rust-lang/crates.io-index",
                    "manifest_path": "/registry/serde/Cargo.toml",
                    "targets": []
                }
            ],
            "resolve": {
                "nodes": [
                    { "id": "client 0.1.0", "dependencies": ["shared 0.1.0", "serde 1.0.0"] },
                    { "id": "shared 0.1.0", "dependencies": [] },
                    { "id": "server 0.1.0", "dependencies": ["shared 0.1.0"] },
                    { "id": "serde 1.0.0", "dependencies": [] }
                ]
            },
            "workspace_root": ws
        });

        let dep_files =
            collect_dep_files_from_metadata_json(json.to_string().as_bytes(), "client 0.1.0")
                .unwrap();

        assert!(dep_files.contains(&ws.join("Cargo.toml")));
        assert!(dep_files.contains(&ws.join("Cargo.lock")));
        assert!(dep_files.contains(&client.join("Cargo.toml")));
        assert!(dep_files.contains(&client.join("src")));
        assert!(dep_files.contains(&client.join("src/lib.rs")));
        assert!(dep_files.contains(&shared.join("Cargo.toml")));
        assert!(dep_files.contains(&shared.join("src")));
        assert!(dep_files.contains(&shared.join("src/lib.rs")));
        assert!(dep_files.contains(&shared.join("src/nested")));
        assert!(dep_files.contains(&shared.join("src/nested/model.rs")));
        assert!(!dep_files.contains(&unused.join("Cargo.toml")));
        assert!(!dep_files.iter().any(|p| p.starts_with("/registry")));

        let _ = fs::remove_dir_all(&ws);
    }
}
