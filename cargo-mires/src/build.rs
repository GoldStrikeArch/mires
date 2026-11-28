//! Build & check orchestration, Cargo invocation, and artifact publishing.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::{
    check_cfg_expr, BuildCliOptions, DiscoverChunks, MessageFormat, OutputFormat, PanicMode,
    RuntimeMode,
};
use crate::metadata::{self, PackageContext, ResolvedConfig, SelectedTarget};
use crate::npm::{self, NpmPackageInput};
use crate::sysroot::{self, SysrootRequest, ToolchainInfo};
use crate::wrapper;

#[derive(Debug, Deserialize)]
struct CargoMessage {
    reason: String,
    #[serde(default)]
    package_id: String,
    #[serde(default)]
    target: Option<CargoArtifactTarget>,
    #[serde(default)]
    filenames: Vec<PathBuf>,
    #[serde(default)]
    executable: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct CargoArtifactTarget {
    name: String,
    #[serde(default)]
    kind: Vec<String>,
}

#[derive(Clone, Debug)]
struct EmittedFile {
    path: PathBuf,
    relative_name: String,
    bytes: u64,
}

/// Structured chunk metadata emitted in `--message-format=json`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JsonChunkInfo {
    pub name: String,
    pub role: String,
    pub path: PathBuf,
    pub source_map: Option<PathBuf>,
    pub dts: Option<PathBuf>,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Clone, Debug)]
struct PublishedArtifacts {
    emitted: Vec<EmittedFile>,
    program: PathBuf,
    source_map: Option<PathBuf>,
    dts: Option<PathBuf>,
    runtime_file: Option<PathBuf>,
    package_json: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    manifest: Option<serde_json::Value>,
    chunks: Vec<JsonChunkInfo>,
}

#[derive(Clone, Debug, Serialize)]
struct BuildJsonOutput {
    schema: u32,
    package: String,
    profile: String,
    format: String,
    runtime: String,
    out_dir: PathBuf,
    program: PathBuf,
    source_map: Option<PathBuf>,
    dts: Option<PathBuf>,
    runtime_file: Option<PathBuf>,
    package_json: Option<PathBuf>,
    manifest_path: Option<PathBuf>,
    manifest: Option<serde_json::Value>,
    chunks: Vec<JsonChunkInfo>,
    dep_files: Vec<PathBuf>,
    elapsed_secs: f64,
}

/// Run `cargo mires build`: ensure sysroot, invoke `cargo rustc`, publish artifacts.
pub fn run_build(opts: &BuildCliOptions) -> Result<(), String> {
    let started = Instant::now();
    let ctx = metadata::load_package_context(opts)?;
    let toolchain = sysroot::ensure_sysroot(&SysrootRequest {
        mires_root_override: opts.mires_root.as_deref(),
        profile: ctx.config.profile,
        panic: ctx.config.panic,
        clean: false,
        verbose: opts.verbose,
    })?;

    let emit_args = build_emit_args(&ctx.config);
    let link_args = build_link_args(&ctx);

    let record_file = temp_record_path(&ctx.package.name);
    let artifact_path =
        run_cargo_rustc(opts, &ctx, &toolchain, &emit_args, &link_args, &record_file)?;

    let recorded_raw = fs::read_to_string(&record_file)
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| !p.as_os_str().is_empty());
    let _ = fs::remove_file(&record_file);

    let published = publish_artifacts(&ctx, &toolchain, &artifact_path, recorded_raw.as_deref())?;
    let elapsed_secs = started.elapsed().as_secs_f64();

    match ctx.config.message_format {
        MessageFormat::Human => print_build_summary(&ctx, &published.emitted, elapsed_secs),
        MessageFormat::Json => {
            let dep_files = metadata::collect_dep_files(&ctx, opts);
            let output = BuildJsonOutput {
                schema: 1,
                package: ctx.package.name,
                profile: ctx.config.profile.as_str().to_owned(),
                format: ctx.config.format.as_str().to_owned(),
                runtime: ctx.config.runtime.as_str().to_owned(),
                out_dir: ctx.config.out_dir,
                program: published.program,
                source_map: published.source_map,
                dts: published.dts,
                runtime_file: published.runtime_file,
                package_json: published.package_json,
                manifest_path: published.manifest_path,
                manifest: published.manifest,
                chunks: published.chunks,
                dep_files,
                elapsed_secs,
            };
            let json = serde_json::to_string(&output)
                .map_err(|err| format!("failed to serialize build JSON output: {err}"))?;
            println!("{json}");
        }
    }
    Ok(())
}

/// Run `cargo mires check`: ensure sysroot, invoke `cargo check` with `MIRES_WRAPPER_MODE=check`.
pub fn run_check(opts: &BuildCliOptions) -> Result<(), String> {
    let ctx = metadata::load_package_context(opts)?;
    let toolchain = sysroot::ensure_sysroot(&SysrootRequest {
        mires_root_override: opts.mires_root.as_deref(),
        profile: ctx.config.profile,
        panic: ctx.config.panic,
        clean: false,
        verbose: opts.verbose,
    })?;

    let wrapper_exe = env::current_exe()
        .map_err(|err| format!("failed to locate current `cargo-mires` executable: {err}"))?;

    let rustflags_env = target_rustflags_env_name(&toolchain.target_name);
    let existing_rustflags = env::var(&rustflags_env).ok();
    let target_rustflags =
        build_target_rustflags(existing_rustflags.as_deref(), None, &ctx.config.cfgs);

    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{}", toolchain.channel)).arg("check");
    if toolchain.is_json_target {
        cmd.arg("-Z").arg("json-target-spec");
    }
    cmd.arg("--target")
        .arg(&toolchain.target_arg)
        .arg("--manifest-path")
        .arg(&ctx.package.manifest_path)
        .arg("-p")
        .arg(&ctx.package.name);

    match &ctx.target {
        SelectedTarget::Lib { .. } => {
            cmd.arg("--lib");
        }
        SelectedTarget::Bin { name } => {
            cmd.arg("--bin").arg(name);
        }
    }

    append_common_cargo_flags(&mut cmd, opts, &ctx.config);
    cmd.env_remove("RUSTUP_TOOLCHAIN")
        .env("RUSTC_WRAPPER", &wrapper_exe)
        .env(&rustflags_env, &target_rustflags)
        .env(wrapper::ENV_WRAPPER_ACTIVE, "1")
        .env(wrapper::ENV_MODE, "check")
        .env(wrapper::ENV_TARGET, &toolchain.target_arg)
        .env(wrapper::ENV_TARGET_NAME, &toolchain.target_name)
        .env(
            wrapper::ENV_JSON_TARGET,
            if toolchain.is_json_target { "1" } else { "0" },
        )
        .env(wrapper::ENV_SYSROOT, &toolchain.sysroot_dir)
        .env(wrapper::ENV_PANIC, ctx.config.panic.as_str())
        .env(wrapper::ENV_CFGS, ctx.config.cfgs.join("\n"))
        .env(wrapper::ENV_VERBOSE, if opts.verbose { "1" } else { "0" })
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd
        .status()
        .map_err(|err| format!("failed to invoke `cargo check`: {err}"))?;
    if !status.success() {
        return Err(format!("`cargo check` failed with {status}"));
    }
    Ok(())
}

fn build_emit_args(config: &ResolvedConfig) -> Vec<String> {
    let mut args = vec![format!("js-profile={}", config.profile.as_str())];
    if config.panic == PanicMode::Unwind {
        args.push("js-cleanup=off".to_owned());
    }
    args
}

fn build_link_args(ctx: &PackageContext) -> Vec<String> {
    let config = &ctx.config;
    let mut args = vec![
        format!("js-modules={}", config.format.as_str()),
        format!("js-runtime={}", config.runtime.as_str()),
        format!("js-dts={}", if config.dts { "on" } else { "off" }),
        format!(
            "js-source-map={}",
            if config.source_map { "on" } else { "off" }
        ),
        format!("js-workspace-root={}", ctx.workspace_root.display()),
        format!("js-discover-chunks={}", config.discover_chunks.as_str()),
    ];

    if config.format == OutputFormat::Esm && config.runtime == RuntimeMode::External {
        args.push(format!("js-shim-module={}", config.shim_module));
    }

    for chunk in &config.chunks {
        args.push(format!("js-chunk={}:{}", chunk.name, chunk.entry));
        if let Some(role) = &chunk.role {
            args.push(format!("js-chunk-role={}:{role}", chunk.name));
        }
    }
    if let Some(shared) = &config.shared_chunk {
        if !config.chunks.is_empty() || config.discover_chunks != DiscoverChunks::Off {
            args.push(format!("js-chunk-shared={shared}"));
        }
    }

    args
}

fn build_target_rustflags(
    existing: Option<&str>,
    build_fp: Option<&str>,
    cfgs: &[String],
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(existing) = existing.filter(|s| !s.trim().is_empty()) {
        parts.push(existing.to_owned());
    }
    parts.push("--cfg mires --check-cfg=cfg(mires)".to_owned());
    if let Some(fp) = build_fp {
        parts.push(format!("--cfg mires_build_fp=\"{fp}\""));
    }
    for spec in cfgs {
        parts.push(format!("--cfg {spec}"));
        if let Some(expr) = check_cfg_expr(spec) {
            parts.push(format!("--check-cfg={expr}"));
        }
    }
    parts.join(" ")
}

fn run_cargo_rustc(
    opts: &BuildCliOptions,
    ctx: &PackageContext,
    toolchain: &ToolchainInfo,
    emit_args: &[String],
    link_args: &[String],
    record_file: &Path,
) -> Result<PathBuf, String> {
    let wrapper_exe = env::current_exe()
        .map_err(|err| format!("failed to locate current `cargo-mires` executable: {err}"))?;

    let cfgs_joined = ctx.config.cfgs.join("\n");
    let build_fp = fingerprint(&[
        &toolchain.dylib_sha256,
        &toolchain.sysroot_dir.to_string_lossy(),
        ctx.config.panic.as_str(),
        &emit_args.join("\n"),
        &cfgs_joined,
    ]);
    let link_fp = fingerprint(&[&link_args.join("\n")]);

    let rustflags_env = target_rustflags_env_name(&toolchain.target_name);
    let existing_rustflags = env::var(&rustflags_env).ok();
    let target_rustflags = build_target_rustflags(
        existing_rustflags.as_deref(),
        Some(&build_fp),
        &ctx.config.cfgs,
    );

    let mut cmd = Command::new("cargo");
    cmd.arg(format!("+{}", toolchain.channel)).arg("rustc");
    if toolchain.is_json_target {
        cmd.arg("-Z").arg("json-target-spec");
    }
    cmd.arg("--target")
        .arg(&toolchain.target_arg)
        .arg("--message-format=json-render-diagnostics")
        .arg("--manifest-path")
        .arg(&ctx.package.manifest_path)
        .arg("-p")
        .arg(&ctx.package.name);

    match &ctx.target {
        SelectedTarget::Lib { .. } => {
            cmd.arg("--lib").arg("--crate-type").arg("cdylib");
        }
        SelectedTarget::Bin { name } => {
            cmd.arg("--bin").arg(name);
        }
    }

    append_common_cargo_flags(&mut cmd, opts, &ctx.config);
    cmd.arg("--")
        .arg("--cfg")
        .arg(format!("mires_link_fp=\"{link_fp}\""));

    cmd.env_remove("RUSTUP_TOOLCHAIN")
        .env("RUSTC_WRAPPER", &wrapper_exe)
        .env(&rustflags_env, &target_rustflags)
        .env(wrapper::ENV_WRAPPER_ACTIVE, "1")
        .env(wrapper::ENV_MODE, "build")
        .env(wrapper::ENV_TARGET, &toolchain.target_arg)
        .env(wrapper::ENV_TARGET_NAME, &toolchain.target_name)
        .env(
            wrapper::ENV_JSON_TARGET,
            if toolchain.is_json_target { "1" } else { "0" },
        )
        .env(wrapper::ENV_SYSROOT, &toolchain.sysroot_dir)
        .env(wrapper::ENV_BACKEND, &toolchain.dylib_path)
        .env(wrapper::ENV_PRODUCER, &toolchain.dylib_sha256)
        .env(wrapper::ENV_PANIC, ctx.config.panic.as_str())
        .env(wrapper::ENV_EMIT_ARGS, emit_args.join("\n"))
        .env(wrapper::ENV_LINK_ARGS, link_args.join("\n"))
        .env(wrapper::ENV_CFGS, &cfgs_joined)
        .env(wrapper::ENV_JSC_LINK, &toolchain.jsc_link_path)
        .env(wrapper::ENV_RECORD_FILE, record_file)
        .env(wrapper::ENV_VERBOSE, if opts.verbose { "1" } else { "0" })
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());

    let output = cmd
        .output()
        .map_err(|err| format!("failed to invoke `cargo rustc`: {err}"))?;

    if !output.status.success() {
        let _ = fs::remove_file(record_file);
        return Err(format!("`cargo rustc` failed with {}", output.status));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    extract_artifact_from_cargo_messages(&stdout, &ctx.package.id, &ctx.target).ok_or_else(|| {
        format!(
            "`cargo rustc` succeeded but no program artifact was reported for `{}` ({})",
            ctx.package.name,
            ctx.target.name()
        )
    })
}

fn append_common_cargo_flags(cmd: &mut Command, opts: &BuildCliOptions, config: &ResolvedConfig) {
    if config.cargo_release {
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
}

/// Extract the primary compiled program artifact path from Cargo's JSON message stream.
pub fn extract_artifact_from_cargo_messages(
    stdout: &str,
    package_id: &str,
    target: &SelectedTarget,
) -> Option<PathBuf> {
    let mut fallback: Option<PathBuf> = None;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('{') {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<CargoMessage>(trimmed) else {
            continue;
        };
        if msg.reason != "compiler-artifact" {
            continue;
        }

        let pkg_matches = msg.package_id == package_id
            || msg.target.as_ref().is_some_and(|t| t.name == target.name());
        if !pkg_matches {
            continue;
        }

        let target_matches = msg.target.as_ref().is_none_or(|t| match target {
            SelectedTarget::Lib { name } => {
                t.name == *name
                    && t.kind
                        .iter()
                        .any(|k| matches!(k.as_str(), "cdylib" | "dylib" | "lib"))
            }
            SelectedTarget::Bin { name } => t.name == *name && t.kind.iter().any(|k| k == "bin"),
        });

        let candidate = msg.executable.or_else(|| {
            msg.filenames
                .iter()
                .find(|p| is_program_artifact_file(p))
                .cloned()
        });

        if let Some(path) = candidate {
            if msg.package_id == package_id && target_matches {
                return Some(path);
            }
            fallback = Some(path);
        }
    }

    fallback
}

fn is_program_artifact_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("wasm" | "js" | "mjs")
    )
}

fn publish_artifacts(
    ctx: &PackageContext,
    toolchain: &ToolchainInfo,
    artifact_path: &Path,
    recorded_raw: Option<&Path>,
) -> Result<PublishedArtifacts, String> {
    let out_dir = &ctx.config.out_dir;
    fs::create_dir_all(out_dir).map_err(|err| {
        format!(
            "failed to create output directory `{}`: {err}",
            out_dir.display()
        )
    })?;

    let stem = artifact_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            format!(
                "artifact path `{}` has no valid UTF-8 file stem",
                artifact_path.display()
            )
        })?;
    let out_js_name = format!("{stem}.{}", ctx.config.format.extension());
    let out_map_name = format!("{out_js_name}.map");
    let out_dts_name = match ctx.config.format {
        OutputFormat::Esm => format!("{stem}.d.mts"),
        OutputFormat::Script => format!("{stem}.d.ts"),
    };

    let js_bytes = fs::read(artifact_path)
        .map_err(|err| format!("failed to read `{}`: {err}", artifact_path.display()))?;
    let raw_artifact = locate_raw_artifact(artifact_path, recorded_raw, &js_bytes);

    let out_js_path = out_dir.join(&out_js_name);
    let final_js_bytes = if ctx.config.source_map {
        let js_text = String::from_utf8(js_bytes).map_err(|err| {
            format!(
                "emitted JavaScript `{}` is not valid UTF-8: {err}",
                artifact_path.display()
            )
        })?;
        rewrite_source_mapping_url(&js_text, &out_map_name).into_bytes()
    } else {
        js_bytes
    };

    fs::write(&out_js_path, &final_js_bytes)
        .map_err(|err| format!("failed to write `{}`: {err}", out_js_path.display()))?;

    let mut emitted = vec![EmittedFile {
        path: out_js_path.clone(),
        relative_name: out_js_name.clone(),
        bytes: final_js_bytes.len() as u64,
    }];

    let mut published_map: Option<PathBuf> = None;
    if ctx.config.source_map {
        if let Some(src_map) = find_sidecar(&raw_artifact, artifact_path, |p| append_ext(p, ".map"))
        {
            let map_text = fs::read_to_string(&src_map).map_err(|err| {
                format!("failed to read source map `{}`: {err}", src_map.display())
            })?;
            let rewritten = rewrite_source_map_file(&map_text, &out_js_name)?;
            let out_map_path = out_dir.join(&out_map_name);
            fs::write(&out_map_path, rewritten.as_bytes()).map_err(|err| {
                format!(
                    "failed to write source map `{}`: {err}",
                    out_map_path.display()
                )
            })?;
            emitted.push(EmittedFile {
                path: out_map_path.clone(),
                relative_name: out_map_name,
                bytes: rewritten.len() as u64,
            });
            published_map = Some(out_map_path);
        }
    }

    let mut published_dts: Option<PathBuf> = None;
    let mut emitted_dts_name: Option<String> = None;
    if ctx.config.dts {
        if let Some(src_dts) =
            find_sidecar(&raw_artifact, artifact_path, |p| p.with_extension("d.ts"))
        {
            let bytes = fs::read(&src_dts)
                .map_err(|err| format!("failed to read `{}`: {err}", src_dts.display()))?;
            let out_dts_path = out_dir.join(&out_dts_name);
            fs::write(&out_dts_path, &bytes)
                .map_err(|err| format!("failed to write `{}`: {err}", out_dts_path.display()))?;
            emitted.push(EmittedFile {
                path: out_dts_path.clone(),
                relative_name: out_dts_name.clone(),
                bytes: bytes.len() as u64,
            });
            published_dts = Some(out_dts_path);
            emitted_dts_name = Some(out_dts_name);
        }
    }

    let manifest_src = find_artifact_manifest(&raw_artifact, artifact_path);

    let mut runtime_file: Option<PathBuf> = None;
    let mut runtime_rel_name: Option<String> = None;
    if ctx.config.runtime == RuntimeMode::External {
        if let Some(shim_rel) = external_shim_relative_path(&ctx.config) {
            let manifest_path = manifest_src.as_ref().ok_or_else(|| {
                format!(
                    "external runtime requested, but no artifact manifest was found beside `{}`",
                    raw_artifact.display()
                )
            })?;
            let out_shim_path = publish_external_runtime_shim(
                &toolchain.jsc_link_path,
                ctx.config.format,
                manifest_path,
                out_dir,
                &shim_rel,
            )?;
            let shim_bytes = fs::metadata(&out_shim_path).map(|m| m.len()).unwrap_or(0);
            emitted.push(EmittedFile {
                path: out_shim_path.clone(),
                relative_name: shim_rel.clone(),
                bytes: shim_bytes,
            });
            runtime_file = Some(out_shim_path);
            runtime_rel_name = Some(shim_rel);
        }
    }

    // Always inspect the linker artifact manifest when present (even when no explicit CLI/metadata
    // chunks were configured) so auto-discovered `#[js_export(chunk = ...)]` outputs and the
    // deployment manifest itself are published to `out_dir`.
    let published_manifest = if let Some(ref src_manifest) = manifest_src {
        publish_manifest_and_chunks(
            src_manifest,
            &raw_artifact,
            stem,
            out_dir,
            runtime_rel_name.as_deref(),
            &mut emitted,
        )?
    } else {
        PublishedManifest::default()
    };

    let mut package_json: Option<PathBuf> = None;
    if ctx.config.npm {
        let rel_files: Vec<String> = emitted.iter().map(|f| f.relative_name.clone()).collect();
        let pkg_path = npm::write_package_json(
            out_dir,
            &NpmPackageInput {
                name: &ctx.package.name,
                version: &ctx.package.version,
                description: ctx.package.description.as_deref(),
                license: ctx.package.license.as_deref(),
                format: ctx.config.format,
                entry_file: &out_js_name,
                dts_file: emitted_dts_name.as_deref(),
                files: &rel_files,
            },
        )?;
        let pkg_bytes = fs::metadata(&pkg_path).map(|m| m.len()).unwrap_or(0);
        emitted.push(EmittedFile {
            path: pkg_path.clone(),
            relative_name: "package.json".to_owned(),
            bytes: pkg_bytes,
        });
        package_json = Some(pkg_path);
    }

    Ok(PublishedArtifacts {
        emitted,
        program: out_js_path,
        source_map: published_map,
        dts: published_dts,
        runtime_file,
        package_json,
        manifest_path: published_manifest.manifest_path,
        manifest: published_manifest.manifest,
        chunks: published_manifest.chunks,
    })
}

fn publish_external_runtime_shim(
    jsc_link: &Path,
    format: OutputFormat,
    manifest_path: &Path,
    out_dir: &Path,
    shim_rel: &str,
) -> Result<PathBuf, String> {
    let manifest_dir = manifest_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let staged_shim_path = manifest_dir.join(shim_rel);
    emit_external_runtime(jsc_link, format, manifest_path, &staged_shim_path)?;

    let out_shim_path = out_dir.join(shim_rel);
    if let Some(parent) = out_shim_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create `{}`: {err}", parent.display()))?;
    }
    if !same_existing_file(&staged_shim_path, &out_shim_path) {
        fs::copy(&staged_shim_path, &out_shim_path).map_err(|err| {
            format!(
                "failed to copy emitted runtime `{}` to `{}`: {err}",
                staged_shim_path.display(),
                out_shim_path.display()
            )
        })?;
    }
    Ok(out_shim_path)
}

fn same_existing_file(a: &Path, b: &Path) -> bool {
    b.exists() && fs::canonicalize(a).ok() == fs::canonicalize(b).ok()
}

fn emit_external_runtime(
    jsc_link: &Path,
    format: OutputFormat,
    manifest_path: &Path,
    out_shim_path: &Path,
) -> Result<(), String> {
    let mut cmd = Command::new(jsc_link);
    cmd.arg("--emit-runtime");
    if format == OutputFormat::Esm {
        cmd.arg("--module");
    }
    cmd.arg("--manifest")
        .arg(manifest_path)
        .arg(out_shim_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().map_err(|err| {
        format!(
            "failed to invoke `{}` for `--emit-runtime`: {err}",
            jsc_link.display()
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "`jsc-link --emit-runtime` failed ({}): {}",
            output.status,
            if stderr.is_empty() {
                "no diagnostic output".to_owned()
            } else {
                stderr
            }
        ));
    }
    Ok(())
}

/// Returns the relative output path when `shim_module` refers to a local `./` file to emit.
///
/// Bare external specifiers (e.g. `topcoat-dom`, `solid-js/web`) return `None` so local shim
/// emission is cleanly skipped.
fn external_shim_relative_path(config: &ResolvedConfig) -> Option<String> {
    let rel = config.shim_module.strip_prefix("./")?;
    (!rel.is_empty()).then(|| rel.to_owned())
}

/// Locate the actual output path `rustc` / `jsc-link` wrote (usually inside `deps/`),
/// where `.map`, `.d.ts`, and `.artifacts.json` sidecars reside.
fn locate_raw_artifact(
    uplifted: &Path,
    recorded_raw: Option<&Path>,
    uplifted_bytes: &[u8],
) -> PathBuf {
    if let Some(raw) = recorded_raw {
        if raw.is_file() {
            return raw.to_path_buf();
        }
    }

    if append_ext(uplifted, ".map").is_file()
        || uplifted.with_extension("d.ts").is_file()
        || append_ext(uplifted, ".artifacts.json").is_file()
        || append_ext(uplifted, ".manifest.json").is_file()
    {
        return uplifted.to_path_buf();
    }

    if let (Some(parent), Some(file_name)) = (uplifted.parent(), uplifted.file_name()) {
        let deps_dir = parent.join("deps");
        let stem = uplifted.file_stem().and_then(|s| s.to_str());

        // Prefer bare `<stem>` in `deps/` when it has sidecars or matches bytes
        // (`backend/src/link.rs` writes `outputs.path(Exe)` without `.wasm`).
        if let Some(stem) = stem {
            let bare_candidates = [deps_dir.join(stem), deps_dir.join(stem.replace('-', "_"))];
            for candidate in bare_candidates {
                if candidate.is_file()
                    && (has_any_sidecar(&candidate)
                        || fs::read(&candidate).is_ok_and(|bytes| bytes == uplifted_bytes))
                {
                    return candidate;
                }
            }
        }

        let direct_dep = deps_dir.join(file_name);
        if direct_dep.is_file() && has_any_sidecar(&direct_dep) {
            return direct_dep;
        }

        if let Some(stem) = stem {
            let ext = uplifted.extension().and_then(|s| s.to_str());
            let prefix = format!("{stem}-");
            let crate_prefix = format!("{}-", stem.replace('-', "_"));
            if let Ok(entries) = fs::read_dir(&deps_dir) {
                let mut fallback_match: Option<PathBuf> = None;
                for entry in entries.flatten() {
                    let candidate = entry.path();
                    let name = candidate.file_name().and_then(|n| n.to_str());
                    let matches_prefix = name
                        .is_some_and(|n| n.starts_with(&prefix) || n.starts_with(&crate_prefix));
                    if !matches_prefix {
                        continue;
                    }
                    let cand_ext = candidate.extension().and_then(|e| e.to_str());
                    let is_bare_or_matching_ext = cand_ext.is_none() || cand_ext == ext;
                    if !is_bare_or_matching_ext || !candidate.is_file() {
                        continue;
                    }
                    if fs::read(&candidate).is_ok_and(|bytes| bytes == uplifted_bytes) {
                        if has_any_sidecar(&candidate) {
                            return candidate;
                        }
                        if cand_ext.is_none() || fallback_match.is_none() {
                            fallback_match = Some(candidate);
                        }
                    }
                }
                if let Some(found) = fallback_match {
                    return found;
                }
            }
        }

        if direct_dep.is_file() {
            return direct_dep;
        }
    }

    uplifted.to_path_buf()
}

fn has_any_sidecar(path: &Path) -> bool {
    append_ext(path, ".map").is_file()
        || path.with_extension("d.ts").is_file()
        || append_ext(path, ".artifacts.json").is_file()
        || append_ext(path, ".manifest.json").is_file()
}

fn find_sidecar<F>(raw: &Path, uplifted: &Path, derive: F) -> Option<PathBuf>
where
    F: Fn(&Path) -> PathBuf,
{
    let primary = derive(raw);
    if primary.is_file() {
        return Some(primary);
    }
    let fallback = derive(uplifted);
    fallback.is_file().then_some(fallback)
}

fn find_artifact_manifest(raw: &Path, uplifted: &Path) -> Option<PathBuf> {
    [
        append_ext(raw, ".artifacts.json"),
        append_ext(raw, ".manifest.json"),
        append_ext(uplifted, ".artifacts.json"),
        append_ext(uplifted, ".manifest.json"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

#[derive(Default)]
struct PublishedManifest {
    manifest_path: Option<PathBuf>,
    manifest: Option<serde_json::Value>,
    chunks: Vec<JsonChunkInfo>,
}

fn publish_manifest_and_chunks(
    manifest_src: &Path,
    raw_artifact: &Path,
    stem: &str,
    out_dir: &Path,
    runtime_rel_name: Option<&str>,
    emitted: &mut Vec<EmittedFile>,
) -> Result<PublishedManifest, String> {
    let raw_bytes = fs::read(manifest_src)
        .map_err(|err| format!("failed to read `{}`: {err}", manifest_src.display()))?;
    let parsed: serde_json::Value = serde_json::from_slice(&raw_bytes)
        .map_err(|err| format!("failed to parse `{}`: {err}", manifest_src.display()))?;

    let out_manifest_name = format!("{stem}.artifacts.json");
    let out_manifest_path = out_dir.join(&out_manifest_name);
    if !same_existing_file(manifest_src, &out_manifest_path) {
        fs::write(&out_manifest_path, &raw_bytes).map_err(|err| {
            format!(
                "failed to write manifest `{}`: {err}",
                out_manifest_path.display()
            )
        })?;
    }
    emitted.push(EmittedFile {
        path: out_manifest_path.clone(),
        relative_name: out_manifest_name,
        bytes: raw_bytes.len() as u64,
    });

    let Some(src_dir) = manifest_src.parent() else {
        return Ok(PublishedManifest {
            manifest_path: Some(out_manifest_path),
            manifest: Some(parsed),
            chunks: Vec::new(),
        });
    };

    let raw_name = raw_artifact.file_name().and_then(|n| n.to_str());
    let raw_map_name = raw_name.map(|n| format!("{n}.map"));
    let raw_dts_name = raw_artifact
        .with_extension("d.ts")
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned);

    let mut copied: BTreeSet<String> = emitted.iter().map(|f| f.relative_name.clone()).collect();

    if let Some(artifacts) = parsed.get("artifacts").and_then(|v| v.as_object()) {
        for (name, artifact_val) in artifacts {
            if Some(name.as_str()) == raw_name
                || raw_map_name.as_deref() == Some(name.as_str())
                || raw_dts_name.as_deref() == Some(name.as_str())
                || runtime_rel_name == Some(name.as_str())
            {
                continue;
            }
            copy_extra_artifact(src_dir, out_dir, name, &mut copied, emitted)?;

            if let Some(map_name) = artifact_val.get("source_map").and_then(|v| v.as_str()) {
                copy_extra_artifact(src_dir, out_dir, map_name, &mut copied, emitted)?;
            }
            if let Some(dts_name) = artifact_val.get("declarations").and_then(|v| v.as_str()) {
                copy_extra_artifact(src_dir, out_dir, dts_name, &mut copied, emitted)?;
            }
        }
    }

    let chunks = extract_published_chunks(&parsed, raw_name, out_dir);
    Ok(PublishedManifest {
        manifest_path: Some(out_manifest_path),
        manifest: Some(parsed),
        chunks,
    })
}

fn copy_extra_artifact(
    src_dir: &Path,
    out_dir: &Path,
    rel_name: &str,
    copied: &mut BTreeSet<String>,
    emitted: &mut Vec<EmittedFile>,
) -> Result<(), String> {
    if !copied.insert(rel_name.to_owned()) {
        return Ok(());
    }
    let src = src_dir.join(rel_name);
    if !src.is_file() {
        return Ok(());
    }
    let dst = out_dir.join(rel_name);
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create `{}`: {err}", parent.display()))?;
    }
    let bytes =
        fs::read(&src).map_err(|err| format!("failed to read `{}`: {err}", src.display()))?;
    if !same_existing_file(&src, &dst) {
        fs::write(&dst, &bytes)
            .map_err(|err| format!("failed to write `{}`: {err}", dst.display()))?;
    }
    emitted.push(EmittedFile {
        path: dst,
        relative_name: rel_name.to_owned(),
        bytes: bytes.len() as u64,
    });
    Ok(())
}

fn extract_published_chunks(
    manifest: &serde_json::Value,
    raw_name: Option<&str>,
    out_dir: &Path,
) -> Vec<JsonChunkInfo> {
    let Some(entries) = manifest.get("entries").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let artifacts = manifest.get("artifacts").and_then(|v| v.as_object());

    let mut chunks = Vec::new();
    for (name, entry_val) in entries {
        let Some(prog_rel) = entry_val.get("program").and_then(|v| v.as_str()) else {
            continue;
        };
        if raw_name == Some(prog_rel) {
            continue;
        }
        let path = out_dir.join(prog_rel);
        if !path.is_file() {
            continue;
        }
        let role = entry_val
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("main")
            .to_owned();
        let art = artifacts.and_then(|map| map.get(prog_rel));

        let source_map = art
            .and_then(|a| a.get("source_map"))
            .and_then(|v| v.as_str())
            .map(|rel| out_dir.join(rel))
            .filter(|p| p.is_file())
            .or_else(|| {
                let candidate = append_ext(&path, ".map");
                candidate.is_file().then_some(candidate)
            });

        let dts = art
            .and_then(|a| a.get("declarations"))
            .and_then(|v| v.as_str())
            .map(|rel| out_dir.join(rel))
            .filter(|p| p.is_file())
            .or_else(|| {
                let candidate = path.with_extension("d.ts");
                candidate.is_file().then_some(candidate)
            });

        let (bytes, sha256) = match (
            art.and_then(|a| a.get("bytes")).and_then(|v| v.as_u64()),
            art.and_then(|a| a.get("sha256")).and_then(|v| v.as_str()),
        ) {
            (Some(b), Some(h)) => (b, h.to_owned()),
            _ => match fs::read(&path) {
                Ok(raw) => (raw.len() as u64, format!("{:x}", Sha256::digest(&raw))),
                Err(_) => (0, String::new()),
            },
        };

        chunks.push(JsonChunkInfo {
            name: name.clone(),
            role,
            path,
            source_map,
            dts,
            sha256,
            bytes,
        });
    }
    chunks
}

fn append_ext(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

/// Rewrite a trailing `//# sourceMappingURL=...` comment to point at `new_map_name`.
pub fn rewrite_source_mapping_url(js: &str, new_map_name: &str) -> String {
    const PREFIX: &str = "//# sourceMappingURL=";
    let trimmed = js.trim_end_matches(['\r', '\n']);
    let trailing_newlines = &js[trimmed.len()..];

    let Some(last_line_start) = trimmed.rfind('\n').map_or(Some(0), |pos| Some(pos + 1)) else {
        return js.to_owned();
    };
    let last_line = &trimmed[last_line_start..];
    if !last_line.starts_with(PREFIX) {
        return js.to_owned();
    }

    let mut out = String::with_capacity(js.len() + new_map_name.len());
    out.push_str(&trimmed[..last_line_start]);
    out.push_str(PREFIX);
    out.push_str(new_map_name);
    out.push_str(trailing_newlines);
    out
}

/// Rewrite the top-level `"file"` field in a source map JSON document to `new_js_file`.
pub fn rewrite_source_map_file(map_json: &str, new_js_file: &str) -> Result<String, String> {
    let mut value: serde_json::Value = serde_json::from_str(map_json)
        .map_err(|err| format!("failed to parse source map JSON: {err}"))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "file".to_owned(),
            serde_json::Value::String(new_js_file.to_owned()),
        );
    }
    serde_json::to_string(&value)
        .map_err(|err| format!("failed to serialize updated source map JSON: {err}"))
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

fn temp_record_path(package_name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let safe_name: String = package_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    env::temp_dir().join(format!(
        "cargo-mires-record-{safe_name}-{}-{nanos}.txt",
        std::process::id()
    ))
}

fn print_build_summary(ctx: &PackageContext, emitted: &[EmittedFile], elapsed_secs: f64) {
    println!(
        "Built `{}` ({}, {} runtime, profile={}) in {elapsed_secs:.2}s:",
        ctx.package.name, ctx.config.format, ctx.config.runtime, ctx.config.profile
    );
    let max_width = emitted
        .iter()
        .map(|f| f.path.display().to_string().len())
        .max()
        .unwrap_or(0);
    for file in emitted {
        let display = file.path.display().to_string();
        println!("  {display:<max_width$}  {:>9}", format_size(file.bytes));
    }
}

/// Format a byte count cleanly in KB (always readable at a glance).
pub fn format_size(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    format!("{kb:.2} KB")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_trailing_source_mapping_url_preserving_body_and_newline() {
        let js = "\
const msg = \"//# sourceMappingURL=literal_in_string.map\";
export function add(a, b) { return a + b; }
//# sourceMappingURL=my_crate.wasm.map
";
        let rewritten = rewrite_source_mapping_url(js, "my_crate.mjs.map");
        assert!(rewritten.contains("literal_in_string.map"));
        assert!(rewritten.ends_with("//# sourceMappingURL=my_crate.mjs.map\n"));
        assert!(!rewritten.contains("my_crate.wasm.map"));
    }

    #[test]
    fn rewrites_source_mapping_url_without_trailing_newline_and_leaves_absent_comment_alone() {
        let no_nl = "export const x = 1;\n//# sourceMappingURL=app-1234.wasm.map";
        assert_eq!(
            rewrite_source_mapping_url(no_nl, "app.mjs.map"),
            "export const x = 1;\n//# sourceMappingURL=app.mjs.map"
        );

        let no_comment = "export const x = 1;\n";
        assert_eq!(
            rewrite_source_mapping_url(no_comment, "app.mjs.map"),
            no_comment
        );
    }

    #[test]
    fn rewrites_file_field_inside_source_map_json() {
        let raw =
            r#"{"version":3,"file":"my_crate.wasm","sources":["src/lib.rs"],"mappings":"AAAA"}"#;
        let updated = rewrite_source_map_file(raw, "my_crate.mjs").unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(parsed["version"], 3);
        assert_eq!(parsed["file"], "my_crate.mjs");
        assert_eq!(parsed["sources"], serde_json::json!(["src/lib.rs"]));
        assert_eq!(parsed["mappings"], "AAAA");
    }

    #[test]
    fn extracts_cdylib_and_bin_artifacts_from_cargo_json_messages() {
        let stream = concat!(
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"dep 0.1.0\",\"target\":{\"name\":\"dep\",\"kind\":[\"rlib\"]},\"filenames\":[\"/t/deps/libdep.rlib\"],\"executable\":null}\n",
            "{\"reason\":\"compiler-artifact\",\"package_id\":\"app 0.1.0\",\"target\":{\"name\":\"app\",\"kind\":[\"cdylib\"]},\"filenames\":[\"/t/mires32-unwind/release/app.wasm\"],\"executable\":null}\n",
            "{\"reason\":\"build-finished\",\"success\":true}\n"
        );

        let found = extract_artifact_from_cargo_messages(
            stream,
            "app 0.1.0",
            &SelectedTarget::Lib {
                name: "app".to_owned(),
            },
        );
        assert_eq!(
            found,
            Some(PathBuf::from("/t/mires32-unwind/release/app.wasm"))
        );

        let bin_stream = "{\"reason\":\"compiler-artifact\",\"package_id\":\"cli 0.1.0\",\"target\":{\"name\":\"my-cli\",\"kind\":[\"bin\"]},\"filenames\":[\"/t/my-cli.wasm\"],\"executable\":\"/t/my-cli.wasm\"}\n";
        let bin_found = extract_artifact_from_cargo_messages(
            bin_stream,
            "cli 0.1.0",
            &SelectedTarget::Bin {
                name: "my-cli".to_owned(),
            },
        );
        assert_eq!(bin_found, Some(PathBuf::from("/t/my-cli.wasm")));
    }

    #[test]
    fn computes_cargo_target_rustflags_env_var_name() {
        assert_eq!(
            target_rustflags_env_name("mires32-unwind"),
            "CARGO_TARGET_MIRES32_UNWIND_RUSTFLAGS"
        );
        assert_eq!(
            target_rustflags_env_name("wasm32-unknown-unknown"),
            "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUSTFLAGS"
        );
    }

    #[test]
    fn locates_hyphenated_binary_raw_artifact_in_deps_on_cached_builds() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = env::temp_dir().join(format!("cargo-mires-raw-{}-{nanos}", std::process::id()));
        let deps = root.join("deps");
        fs::create_dir_all(&deps).unwrap();

        let body = b"export function main() {}\n";
        let uplifted = root.join("my-app.wasm");
        let bare_dep = deps.join("my_app-deadbeef");
        let wasm_dep = deps.join("my_app-deadbeef.wasm");
        fs::write(&uplifted, body).unwrap();
        fs::write(&bare_dep, body).unwrap();
        fs::write(deps.join("my_app-deadbeef.map"), "{}").unwrap();
        fs::write(&wasm_dep, body).unwrap();

        let located = locate_raw_artifact(&uplifted, None, body);
        assert_eq!(located, bare_dep);

        let _ = fs::remove_dir_all(&root);
    }

    fn sample_context(out_dir: PathBuf) -> PackageContext {
        PackageContext {
            workspace_root: PathBuf::from("/ws"),
            target_directory: PathBuf::from("/ws/target"),
            package: metadata::PackageInfo {
                id: "demo 0.1.0".to_owned(),
                name: "demo".to_owned(),
                version: "0.1.0".to_owned(),
                description: None,
                license: None,
                edition: "2021".to_owned(),
                manifest_path: PathBuf::from("/ws/demo/Cargo.toml"),
                manifest_dir: PathBuf::from("/ws/demo"),
            },
            target: SelectedTarget::Lib {
                name: "demo".to_owned(),
            },
            config: ResolvedConfig {
                profile: crate::cli::Profile::Dev,
                cargo_release: false,
                format: OutputFormat::Esm,
                runtime: RuntimeMode::Inline,
                dts: true,
                source_map: true,
                npm: false,
                panic: PanicMode::Unwind,
                out_dir,
                shim_module: "./shim.mjs".to_owned(),
                chunks: Vec::new(),
                shared_chunk: None,
                discover_chunks: DiscoverChunks::Auto,
                cfgs: Vec::new(),
                message_format: MessageFormat::Human,
            },
        }
    }

    #[test]
    fn build_link_args_emits_discover_chunks_roles_shared_and_shim_module() {
        let mut ctx = sample_context(PathBuf::from("/ws/dist"));
        ctx.config.runtime = RuntimeMode::External;
        ctx.config.shim_module = "topcoat-dom".to_owned();
        ctx.config.discover_chunks = DiscoverChunks::On;
        ctx.config.shared_chunk = Some("common".to_owned());
        ctx.config.chunks = vec![
            metadata::ChunkRequest {
                name: "app".to_owned(),
                entry: "boot".to_owned(),
                role: Some("main".to_owned()),
            },
            metadata::ChunkRequest {
                name: "counter".to_owned(),
                entry: "__island_counter".to_owned(),
                role: Some("island".to_owned()),
            },
        ];

        let args = build_link_args(&ctx);
        assert!(args.contains(&"js-discover-chunks=on".to_owned()));
        assert!(args.contains(&"js-shim-module=topcoat-dom".to_owned()));
        assert!(args.contains(&"js-chunk=app:boot".to_owned()));
        assert!(args.contains(&"js-chunk-role=app:main".to_owned()));
        assert!(args.contains(&"js-chunk=counter:__island_counter".to_owned()));
        assert!(args.contains(&"js-chunk-role=counter:island".to_owned()));
        assert!(args.contains(&"js-chunk-shared=common".to_owned()));
    }

    #[test]
    fn target_rustflags_includes_custom_cfgs_and_check_cfgs() {
        let flags = build_target_rustflags(
            Some("-C opt-level=2"),
            Some("deadbeef"),
            &["topcoat_client".to_owned(), "mode=\"csr\"".to_owned()],
        );
        assert!(flags.starts_with("-C opt-level=2 "));
        assert!(flags.contains("--cfg mires --check-cfg=cfg(mires)"));
        assert!(flags.contains("--cfg mires_build_fp=\"deadbeef\""));
        assert!(flags.contains("--cfg topcoat_client --check-cfg=cfg(topcoat_client)"));
        assert!(flags.contains("--cfg mode=\"csr\" --check-cfg=cfg(mode,values(any()))"));
    }

    #[test]
    fn bare_external_shim_specifier_skips_local_shim_emission() {
        let mut ctx = sample_context(PathBuf::from("/ws/dist"));
        ctx.config.runtime = RuntimeMode::External;
        ctx.config.shim_module = "./runtime.mjs".to_owned();
        assert_eq!(
            external_shim_relative_path(&ctx.config).as_deref(),
            Some("runtime.mjs")
        );

        ctx.config.shim_module = "topcoat-dom".to_owned();
        assert_eq!(external_shim_relative_path(&ctx.config), None);

        ctx.config.format = OutputFormat::Script;
        ctx.config.shim_module = "solid-js/web".to_owned();
        assert_eq!(external_shim_relative_path(&ctx.config), None);
    }

    #[test]
    fn publishes_auto_discovered_chunks_and_manifest_with_bare_external_shim() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "cargo-mires-publish-auto-{}-{nanos}",
            std::process::id()
        ));
        let deps = root.join("deps");
        let out_dir = root.join("dist");
        fs::create_dir_all(&deps).unwrap();

        let primary_js = b"export function main() {}\n//# sourceMappingURL=demo-1234.map\n";
        let raw_artifact = deps.join("demo-1234");
        let uplifted = root.join("demo.wasm");
        fs::write(&raw_artifact, primary_js).unwrap();
        fs::write(&uplifted, primary_js).unwrap();
        fs::write(
            deps.join("demo-1234.map"),
            r#"{"version":3,"file":"demo-1234","sources":[],"mappings":""}"#,
        )
        .unwrap();
        fs::write(
            deps.join("demo-1234.d.ts"),
            "export function main(): void;\n",
        )
        .unwrap();

        let island_bytes = b"export function __island_counter() {}\n";
        let shared_bytes = b"export const SHARED = 1;\n";
        fs::write(deps.join("counter.js"), island_bytes).unwrap();
        fs::write(deps.join("counter.js.map"), r#"{"version":3}"#).unwrap();
        fs::write(
            deps.join("counter.d.ts"),
            "export function __island_counter(): void;\n",
        )
        .unwrap();
        fs::write(deps.join("shared.js"), shared_bytes).unwrap();

        let island_sha = format!("{:x}", Sha256::digest(island_bytes));
        let shared_sha = format!("{:x}", Sha256::digest(shared_bytes));
        let manifest_json = serde_json::json!({
            "schema": 2,
            "entries": {
                "demo": { "role": "main", "program": "demo-1234", "roots": {} },
                "counter": { "role": "island", "program": "counter.js", "roots": {} },
                "shared": { "role": "shared", "program": "shared.js", "roots": {} }
            },
            "artifacts": {
                "demo-1234": { "kind": "program", "bytes": primary_js.len(), "sha256": "0".repeat(64) },
                "demo-1234.map": { "kind": "source-map", "bytes": 10, "sha256": "0".repeat(64) },
                "demo-1234.d.ts": { "kind": "declarations", "bytes": 10, "sha256": "0".repeat(64) },
                "counter.js": {
                    "kind": "program",
                    "bytes": island_bytes.len(),
                    "sha256": island_sha,
                    "source_map": "counter.js.map",
                    "declarations": "counter.d.ts"
                },
                "counter.js.map": { "kind": "source-map", "bytes": 13, "sha256": "0".repeat(64) },
                "counter.d.ts": { "kind": "declarations", "bytes": 41, "sha256": "0".repeat(64) },
                "shared.js": {
                    "kind": "program",
                    "bytes": shared_bytes.len(),
                    "sha256": shared_sha
                }
            }
        });
        fs::write(
            deps.join("demo-1234.artifacts.json"),
            serde_json::to_vec_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        // No explicit chunks configured; runtime is External with bare specifier (`topcoat-dom`).
        let mut ctx = sample_context(out_dir.clone());
        ctx.config.runtime = RuntimeMode::External;
        ctx.config.shim_module = "topcoat-dom".to_owned();
        let toolchain = ToolchainInfo {
            channel: "nightly".to_owned(),
            dylib_path: PathBuf::new(),
            dylib_sha256: String::new(),
            jsc_link_path: PathBuf::from("/nonexistent/jsc-link"),
            sysroot_dir: PathBuf::new(),
            target_arg: "mires32-unwind".to_owned(),
            target_name: "mires32-unwind".to_owned(),
            is_json_target: false,
        };

        let published =
            publish_artifacts(&ctx, &toolchain, &uplifted, Some(&raw_artifact)).unwrap();
        assert_eq!(published.program, out_dir.join("demo.mjs"));
        assert_eq!(published.source_map, Some(out_dir.join("demo.mjs.map")));
        assert_eq!(published.dts, Some(out_dir.join("demo.d.mts")));
        assert_eq!(published.runtime_file, None);
        assert_eq!(
            published.manifest_path,
            Some(out_dir.join("demo.artifacts.json"))
        );
        assert!(out_dir.join("demo.artifacts.json").is_file());
        assert!(out_dir.join("counter.js").is_file());
        assert!(out_dir.join("counter.js.map").is_file());
        assert!(out_dir.join("counter.d.ts").is_file());
        assert!(out_dir.join("shared.js").is_file());

        assert_eq!(
            published.chunks,
            vec![
                JsonChunkInfo {
                    name: "counter".to_owned(),
                    role: "island".to_owned(),
                    path: out_dir.join("counter.js"),
                    source_map: Some(out_dir.join("counter.js.map")),
                    dts: Some(out_dir.join("counter.d.ts")),
                    sha256: island_sha,
                    bytes: island_bytes.len() as u64,
                },
                JsonChunkInfo {
                    name: "shared".to_owned(),
                    role: "shared".to_owned(),
                    path: out_dir.join("shared.js"),
                    source_map: None,
                    dts: None,
                    sha256: shared_sha,
                    bytes: shared_bytes.len() as u64,
                },
            ]
        );

        let _ = fs::remove_dir_all(&root);
    }
}
