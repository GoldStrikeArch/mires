//! The `jsc-link` binary.
//!
//! `jsc-link --manifest <path>` runs one link step and exits non-zero if it reported an error;
//! `jsc-link --print-manifest <path>` dumps a manifest for inspection. The dylib streams this
//! process's stderr through unchanged, so the messages a user sees are the ones written here.

use std::process::ExitCode;

use rcgjs_object::manifest::{Manifest, MANIFEST_VERSION};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (flag, path) = match args.as_slice() {
        [flag, path] => (flag.as_str(), path.as_str()),
        _ => {
            eprintln!("usage: jsc-link --manifest <path> | --print-manifest <path> | --verify-artifacts <path>");
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
