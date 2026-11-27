//! The `jsc-link` binary.
//!
//! `jsc-link --manifest <path>` runs one link step and exits non-zero if it reported an error;
//! `jsc-link --print-manifest <path>` dumps a manifest for inspection;
//! `jsc-link --verify-artifacts <path>` checks an emitted artifact manifest;
//! `jsc-link --emit-runtime [--module] [--full] --manifest <manifest.json> <out.js>` extracts a
//! minimal (or full) canonical runtime shim from an artifact manifest. The dylib streams this
//! process's stderr through unchanged, so the messages a user sees are the ones written here.

use std::path::PathBuf;
use std::process::ExitCode;

use rcgjs_object::manifest::{Manifest, MANIFEST_VERSION};

const USAGE: &str = "usage: jsc-link --manifest <path> | --print-manifest <path> | \
    --verify-artifacts <path> | --emit-runtime [--module] [--full] --manifest <manifest.json> <out.js>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--emit-runtime") {
        return run_emit_runtime(&args[1..]);
    }

    let (flag, path) = match args.as_slice() {
        [flag, path] => (flag.as_str(), path.as_str()),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };

    if flag == "--verify-artifacts" {
        let path = std::path::Path::new(path);
        let directory = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(std::path::Path::new("."));
        return match rcgjs_object::artifact::ArtifactManifest::read(path)
            .and_then(|manifest| manifest.verify(directory))
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!(
                    "error: artifact verification of `{}` failed: {error}",
                    path.display()
                );
                ExitCode::FAILURE
            }
        };
    }

    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("error: reading `{path}`: {err}");
            return ExitCode::FAILURE;
        }
    };
    let manifest: Manifest = match postcard::from_bytes(&bytes) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("error: `{path}` is not a jsc-link manifest: {err}");
            return ExitCode::FAILURE;
        }
    };
    if manifest.version != MANIFEST_VERSION {
        eprintln!(
            "error: `{path}` is a version {} manifest; this jsc-link reads version {}",
            manifest.version, MANIFEST_VERSION
        );
        return ExitCode::FAILURE;
    }

    match flag {
        "--manifest" => match jsc_link::run(&manifest) {
            0 => ExitCode::SUCCESS,
            _ => ExitCode::FAILURE,
        },
        "--print-manifest" => {
            println!("{manifest:#?}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("error: unknown option `{flag}`");
            ExitCode::from(2)
        }
    }
}

fn run_emit_runtime(args: &[String]) -> ExitCode {
    let mut module = false;
    let mut full = false;
    let mut manifest_path: Option<PathBuf> = None;
    let mut out_path: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--module" => module = true,
            "--full" => full = true,
            "--manifest" => {
                i += 1;
                let Some(path) = args.get(i) else {
                    eprintln!("error: `--manifest` requires a path");
                    return ExitCode::from(2);
                };
                if manifest_path.is_some() {
                    eprintln!("error: `--manifest` was given more than once");
                    return ExitCode::from(2);
                }
                manifest_path = Some(PathBuf::from(path));
            }
            opt if opt.starts_with("--") => {
                eprintln!("error: unknown option `{opt}`");
                return ExitCode::from(2);
            }
            pos => {
                if out_path.is_some() {
                    eprintln!("error: unexpected positional argument `{pos}`");
                    return ExitCode::from(2);
                }
                out_path = Some(PathBuf::from(pos));
            }
        }
        i += 1;
    }

    let (Some(manifest_path), Some(out_path)) = (manifest_path, out_path) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    match jsc_link::runtime_emit::emit_runtime_from_manifest(
        &manifest_path,
        &out_path,
        module,
        full,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: emit-runtime failed: {error}");
            ExitCode::FAILURE
        }
    }
}
