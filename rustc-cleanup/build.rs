use std::{env, fs, path::PathBuf, process::Command};

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-env-changed=RUSTC");
    println!("cargo:rerun-if-changed=rustc.commit");
    println!("cargo:rerun-if-changed=sources.sha256");
    let rustc = env::var_os("RUSTC").expect("Cargo provides RUSTC");
    let version = Command::new(&rustc)
        .arg("-vV")
        .output()
        .expect("run rustc -vV");
    assert!(version.status.success(), "rustc -vV failed");
    let version = String::from_utf8(version.stdout).expect("rustc version is UTF-8");
    let commit = version
        .lines()
        .find_map(|line| line.strip_prefix("commit-hash: "));
    assert_eq!(
        commit,
        Some(include_str!("rustc.commit").trim()),
        "cleanup bridge requires its audited rustc commit"
    );
    let output = Command::new(rustc)
        .arg("--print=sysroot")
        .output()
        .expect("read rustc sysroot");
    assert!(output.status.success(), "rustc --print=sysroot failed");
    let source = PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("sysroot is UTF-8")
            .trim(),
    )
    .join("lib/rustlib/rustc-src/rust");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo provides OUT_DIR"));
    let mut modules = String::new();
    for entry in include_str!("sources.sha256").lines() {
        let fields: Vec<_> = entry.split_whitespace().collect();
        let [kind, expected, relative] = fields.as_slice() else {
            panic!("invalid cleanup source manifest entry: {entry}");
        };
        let path = source.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "cleanup bridge needs audited rustc sources at {}: {error}",
                path.display()
            )
        });
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            *expected,
            "cleanup source differs from the audited version: {relative}"
        );
        match *kind {
            "module" => {
                let name = path
                    .file_stem()
                    .expect("module has a stem")
                    .to_str()
                    .expect("ASCII stem");
                let target = out.join(format!("{name}.rs"));
                fs::write(&target, bytes).expect("copy audited rustc module");
                modules +=
                    &format!("#[allow(dead_code, clippy::all)] #[path = {target:?}] mod {name};\n");
            }
            "coroutine" => {
                let mut text = String::from_utf8(bytes).expect("rustc source is UTF-8");
                for line in [
                    "mod by_move_body;\n",
                    "pub(super) use by_move_body::coroutine_by_move_body_def_id;\n",
                    "pub(super) use layout::mir_coroutine_witnesses;\n",
                ] {
                    text = replace_once(text, line, "");
                }
                text = replace_once(
                    text,
                    "use crate::{abort_unwinding_calls, pass_manager as pm, simplify};",
                    "use crate::simplify;",
                );
                text = replace_once(
                    text,
                    "    pm::run_passes_no_validate(tcx, body, &[&abort_unwinding_calls::AbortUnwindingCalls], None);",
                    "    // Host exception edges survive in this private JavaScript lowering body.",
                );
                text = replace_once(
                    text,
                    "    // Nothing can unwind when landing pads are off.\n    if !tcx.sess.panic_strategy().unwinds() {\n        return false;\n    }\n",
                    "    // Host exceptions can leave a poll even when Rust panics abort.\n    let _ = tcx;\n",
                );
                let target = out.join("coroutine.rs");
                fs::write(&target, text).expect("write adapted coroutine pass");
                modules += &format!(
                    "#[allow(dead_code, unused_imports, clippy::all)] #[path = {target:?}] mod coroutine;\n"
                );
            }
            "coroutine-drop" => {
                let text = replace_once(
                    String::from_utf8(bytes).expect("rustc source is UTF-8"),
                    "    pm::run_passes_no_validate(\n        tcx,\n        &mut body,\n        &[&abort_unwinding_calls::AbortUnwindingCalls],\n        None,\n    );",
                    "    // Host exception edges survive in this private JavaScript lowering body.",
                );
                fs::write(out.join("drop.rs"), text).expect("write adapted coroutine drops");
            }
            "coroutine-layout" => {
                let text = String::from_utf8(bytes).expect("rustc source is UTF-8");
                // The original provider already performs the witness and lint queries. Keep the
                // unchanged layout algorithm without compiling those unused diagnostic helpers.
                let text = text
                    .split_once("#[instrument(level = \"debug\", skip(tcx), ret)]")
                    .expect("witness function marker")
                    .0;
                let text = text.replace(
                    "use crate::diagnostics::{MustNotSupend, MustNotSuspendReason};",
                    "",
                );
                fs::write(out.join("layout.rs"), text).expect("write coroutine layout prefix");
            }
            "drop-shim" => {
                let text = String::from_utf8(bytes).expect("rustc source is UTF-8");
                let start = text
                    .find("pub(super) struct DropShimElaborator")
                    .expect("drop shim marker");
                let end = text
                    .find("fn build_thread_local_shim")
                    .expect("next shim marker");
                let prefix = "use std::fmt; use rustc_abi::{FieldIdx, VariantIdx}; use rustc_middle::mir::*; use rustc_middle::ty::{self, TyCtxt}; use crate::elaborate_drop::{DropElaborator, DropFlagMode, DropStyle}; use crate::patch::MirPatch;\n";
                let target = out.join("shim.rs");
                fs::write(&target, format!("{prefix}{}", &text[start..end]))
                    .expect("write unchanged drop shim elaborator");
                modules +=
                    &format!("#[allow(dead_code, clippy::all)] #[path = {target:?}] mod shim;\n");
            }
            "runtime-select" => {
                let text = String::from_utf8(bytes).expect("rustc source is UTF-8");
                let start = text
                    .find("fn remap_mir_for_const_eval_select")
                    .expect("runtime selection marker");
                let end = text.find("fn take_array").expect("runtime selection end");
                let prefix = "use rustc_hir as hir; use rustc_middle::mir::*; use rustc_middle::ty::{self, TyCtxt}; use rustc_span::{sym, DUMMY_SP}; use rustc_span::Spanned; use crate::take_array;\n";
                let target = out.join("runtime_select.rs");
                fs::write(&target, format!("{prefix}pub(super) {}", &text[start..end]))
                    .expect("write runtime selection");
                modules +=
                    &format!("#[allow(clippy::all)] #[path = {target:?}] mod runtime_select;\n");
            }
            "audit" => {}
            _ => panic!("invalid cleanup source manifest role: {kind}"),
        }
    }
    fs::write(out.join("modules.rs"), modules).expect("write rustc module declarations");
}

fn replace_once(text: String, from: &str, to: &str) -> String {
    assert_eq!(
        text.matches(from).count(),
        1,
        "audited source patch differs"
    );
    text.replacen(from, to, 1)
}
