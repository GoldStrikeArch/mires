//! Build & check orchestration, Cargo invocation, and artifact publishing.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::cli::{BuildCliOptions, OutputFormat, PanicMode, RuntimeMode};
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

    let emitted = publish_artifacts(&ctx, &toolchain, &artifact_path, recorded_raw.as_deref())?;

    print_build_summary(&ctx, &emitted, started.elapsed().as_secs_f64());
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
    ];

    if config.format == OutputFormat::Esm && config.runtime == RuntimeMode::External {
        args.push(format!("js-shim-module={}", config.shim_module));
    }

    for (name, entry) in &config.chunks {
        args.push(format!("js-chunk={name}:{entry}"));
    }
    if !config.chunks.is_empty() {
        if let Some(shared) = &config.shared_chunk {
            args.push(format!("js-chunk-shared={shared}"));
        }
    }

    args
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

    let build_fp = fingerprint(&[
        &toolchain.dylib_sha256,
        &toolchain.sysroot_dir.to_string_lossy(),
        ctx.config.panic.as_str(),
        &emit_args.join("\n"),
    ]);
    let link_fp = fingerprint(&[&link_args.join("\n")]);

    let rustflags_env = target_rustflags_env_name(&toolchain.target_name);
    let mut target_rustflags = env::var(&rustflags_env).unwrap_or_default();
    if !target_rustflags.is_empty() {
        target_rustflags.push(' ');
    }
    target_rustflags.push_str(&format!(
        "--cfg mires --check-cfg=cfg(mires) --cfg mires_build_fp=\"{build_fp}\""
    ));

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
) -> Result<Vec<EmittedFile>, String> {
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
        path: out_js_path,
        relative_name: out_js_name.clone(),
        bytes: final_js_bytes.len() as u64,
    }];

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
                path: out_map_path,
                relative_name: out_map_name,
                bytes: rewritten.len() as u64,
            });
        }
    }

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
                path: out_dts_path,
                relative_name: out_dts_name.clone(),
                bytes: bytes.len() as u64,
            });
            emitted_dts_name = Some(out_dts_name);
        }
    }

    if !ctx.config.chunks.is_empty() {
        publish_chunk_artifacts(&raw_artifact, artifact_path, out_dir, &mut emitted)?;
    }

    if ctx.config.runtime == RuntimeMode::External {
        if let Some(shim_rel) = external_shim_relative_path(&ctx.config) {
            let manifest_path = find_artifact_manifest(&raw_artifact, artifact_path).ok_or_else(
                || {
                    format!(
                        "external runtime requested, but no artifact manifest was found beside `{}`",
                        raw_artifact.display()
                    )
                },
            )?;
            let manifest_dir = manifest_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            let staged_shim_path = manifest_dir.join(&shim_rel);
            emit_external_runtime(
                &toolchain.jsc_link_path,
                ctx.config.format,
                &manifest_path,
                &staged_shim_path,
            )?;

            let out_shim_path = out_dir.join(&shim_rel);
            if let Some(parent) = out_shim_path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|err| format!("failed to create `{}`: {err}", parent.display()))?;
            }
            let same_dest = fs::canonicalize(&staged_shim_path).ok()
                == fs::canonicalize(&out_shim_path).ok()
                && out_shim_path.exists();
            if !same_dest {
                fs::copy(&staged_shim_path, &out_shim_path).map_err(|err| {
                    format!(
                        "failed to copy emitted runtime `{}` to `{}`: {err}",
                        staged_shim_path.display(),
                        out_shim_path.display()
                    )
                })?;
            }
            let shim_bytes = fs::metadata(&out_shim_path).map(|m| m.len()).unwrap_or(0);
            emitted.push(EmittedFile {
                path: out_shim_path,
                relative_name: shim_rel,
                bytes: shim_bytes,
            });
        }
    }

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
            path: pkg_path,
            relative_name: "package.json".to_owned(),
            bytes: pkg_bytes,
        });
    }

    Ok(emitted)
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

fn external_shim_relative_path(config: &ResolvedConfig) -> Option<String> {
    match config.format {
        OutputFormat::Esm => {
            let rel = config.shim_module.strip_prefix("./")?;
            (!rel.is_empty()).then(|| rel.to_owned())
        }
        OutputFormat::Script => {
            let rel = config.shim_module.trim_start_matches("./");
            if rel.is_empty() {
                Some("shim.js".to_owned())
            } else {
                Some(rel.to_owned())
            }
        }
    }
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

fn publish_chunk_artifacts(
    raw_artifact: &Path,
    uplifted: &Path,
    out_dir: &Path,
    emitted: &mut Vec<EmittedFile>,
) -> Result<(), String> {
    let Some(manifest_path) = find_artifact_manifest(raw_artifact, uplifted) else {
        return Ok(());
    };
    let Some(src_dir) = manifest_path.parent() else {
        return Ok(());
    };
    let raw_bytes = fs::read(&manifest_path)
        .map_err(|err| format!("failed to read `{}`: {err}", manifest_path.display()))?;
    let parsed: serde_json::Value = serde_json::from_slice(&raw_bytes)
        .map_err(|err| format!("failed to parse `{}`: {err}", manifest_path.display()))?;
    let Some(artifacts) = parsed.get("artifacts").and_then(|v| v.as_object()) else {
        return Ok(());
    };

    let raw_name = raw_artifact.file_name().and_then(|n| n.to_str());
    let raw_map_name = raw_name.map(|n| format!("{n}.map"));
    let raw_dts_name = raw_artifact
        .with_extension("d.ts")
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned);

    for name in artifacts.keys() {
        if Some(name.as_str()) == raw_name
            || raw_map_name.as_deref() == Some(name.as_str())
            || raw_dts_name.as_deref() == Some(name.as_str())
        {
            continue;
        }
        let src = src_dir.join(name);
        if !src.is_file() {
            continue;
        }
        let dst = out_dir.join(name);
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("failed to create `{}`: {err}", parent.display()))?;
        }
        let bytes =
            fs::read(&src).map_err(|err| format!("failed to read `{}`: {err}", src.display()))?;
        fs::write(&dst, &bytes)
            .map_err(|err| format!("failed to write `{}`: {err}", dst.display()))?;
        emitted.push(EmittedFile {
            path: dst,
            relative_name: name.clone(),
            bytes: bytes.len() as u64,
        });
    }
    Ok(())
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
}
