//! The producer binary is a tracked input to rustc and executable MIR transport.

use std::path::PathBuf;
use std::sync::Mutex;

use rustc_session::Session;
use sha2::{Digest, Sha256};

static DIGEST: Mutex<Option<[u8; 32]>> = Mutex::new(None);

pub(crate) fn init(sess: &Session) {
    let path = backend_path(sess);
    let bytes = std::fs::read(&path).unwrap_or_else(|error| {
        sess.dcx()
            .fatal(format!("cannot hash backend {}: {error}", path.display()))
    });
    let digest = Sha256::digest(bytes);
    // Hash the file exactly once. The command-line value is its ordinary SHA-256, not a path,
    // timestamp, source-tree estimate, or linker identity.
    let expected = format!("{digest:x}");
    let mut supplied = sess
        .opts
        .cg
        .llvm_args
        .iter()
        .filter_map(|arg| arg.strip_prefix("js-producer="));
    let value = supplied.next();
    if supplied.next().is_some() {
        sess.dcx().fatal("duplicate js-producer identity");
    }
    if let Some(value) = value {
        if value != expected {
            sess.dcx().fatal(format!(
                "js-producer does not match the backend binary; expected {expected}"
            ));
        }
    } else if sess.opts.incremental.is_some() {
        sess.dcx().fatal(format!(
            "incremental JavaScript compilation requires -Cllvm-args=js-producer={expected}; \
             use the compiler build scripts or hash the selected backend before invoking rustc"
        ));
    }
    *DIGEST.lock().expect("producer identity poisoned") = Some(digest.into());
}

/// Resolve the same library as the pinned rustc loader, including a named sysroot backend.
fn backend_path(sess: &Session) -> PathBuf {
    use rustc_session::{config::host_tuple, filesearch::make_target_lib_path};
    use std::env::consts::{DLL_PREFIX, DLL_SUFFIX};

    let name = sess
        .opts
        .unstable_opts
        .codegen_backend
        .as_deref()
        .or(sess.target.default_codegen_backend.as_deref())
        .unwrap_or("js");
    if name.contains('.') {
        return PathBuf::from(name);
    }
    let directory = sess
        .opts
        .sysroot
        .all_paths()
        .map(|root| make_target_lib_path(root, host_tuple()).with_file_name("codegen-backends"))
        .find(|path| path.exists())
        .unwrap_or_else(|| {
            sess.dcx()
                .fatal("cannot locate the loaded JavaScript backend")
        });
    let release = sess.cfg_version.split_whitespace().next().unwrap_or("");
    let names = [
        format!("{DLL_PREFIX}rustc_codegen_{name}{DLL_SUFFIX}"),
        format!("{DLL_PREFIX}rustc_codegen_{name}-{release}{DLL_SUFFIX}"),
    ];
    let candidates: Vec<_> = names
        .iter()
        .map(|name| directory.join(name))
        .filter(|path| path.is_file())
        .collect();
    match candidates.as_slice() {
        [path] => path.clone(),
        _ => sess
            .dcx()
            .fatal("cannot uniquely identify the loaded JavaScript backend"),
    }
}

pub(crate) fn get() -> [u8; 32] {
    DIGEST
        .lock()
        .expect("producer identity poisoned")
        .expect("producer identity initialized")
}
