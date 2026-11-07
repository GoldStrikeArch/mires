//! `rustc_codegen_mires` — an out-of-tree rustc codegen backend that lowers monomorphized MIR to
//! JavaScript.
//!
//! Plug-in mechanics are modeled on `rustc_codegen_cranelift` (`src/lib.rs`): the crate is a
//! `dylib` exposing `__rustc_codegen_backend`, and `codegen_crate` delegates to
//! `rustc_codegen_ssa::base::codegen_crate` so rustc drives monomorphization, partitioning and
//! output-file bookkeeping for us. The "object file" we write is the JavaScript text.

#![feature(rustc_private)]

extern crate rustc_abi;
extern crate rustc_ast;
extern crate rustc_codegen_ssa;
extern crate rustc_data_structures;
extern crate rustc_errors;
extern crate rustc_hashes;
extern crate rustc_hir;
extern crate rustc_metadata;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;
extern crate rustc_symbol_mangling;
extern crate rustc_target;

// Linking against rustc_driver prevents duplicating functions and statics that are already part of
// the host rustc process (cg_clif src/lib.rs:29-31).
#[allow(unused_extern_crates)]
extern crate rustc_driver;

use std::any::Any;

use rustc_codegen_ssa::traits::CodegenBackend;
use rustc_codegen_ssa::{CompiledModules, CrateInfo, TargetConfig};
use rustc_metadata::EncodedMetadata;
use rustc_middle::dep_graph::WorkProductMap;
use rustc_middle::ty::TyCtxt;
use rustc_session::Session;
use rustc_session::config::OutputFilenames;
use rustc_span::{Symbol, sym};
use rustc_target::spec::{Arch, Os};

mod abi;
mod alloc_support;
mod base;
/// The `js!{}` block format.
///
/// Included from `js-macro` for the same reason [`descriptor`] is included from
/// `js-extern-macro`: the macro encodes, this backend decodes, and one file is what keeps the two
/// from drifting. See its own docs for the encoding.
#[path = "../../js-macro/src/block.rs"]
#[allow(dead_code)]
mod block;
mod cfg_adapter;
mod cgu;
mod constant;
mod constread;
/// The `#[js_extern]` descriptor format.
///
/// The file itself lives in `js-extern-macro`, which is where it is written, and is included here
/// rather than copied: the macro encodes and this backend decodes, and two copies of a format is
/// how a format drifts. A proc-macro crate can export nothing but proc macros, so a dependency
/// would have meant a third crate carrying one file.
///
/// The encoding half is the macro's and is unused here, which is what the `dead_code` allowance is
/// for. Its unit tests come along and run under `cargo test -p rustc_codegen_mires` as well, so the
/// two sides are checked against one format rather than against each other.
#[path = "../../js-extern-macro/src/descriptor.rs"]
#[allow(dead_code)]
mod descriptor;
mod driver;
mod dts;
mod emit;
mod intrinsics;
mod item;
mod js_block;
mod js_extern;
mod jsast;
mod link;
mod map;
mod minify;
mod names;
mod naming;
mod opts;
mod place;
mod ptr;
mod queue;
mod rvalue;
mod sourcemap;
mod tag;
mod template;
mod unsize;
mod uses;
mod value;
mod vtable;

pub struct JsCodegenBackend;

impl CodegenBackend for JsCodegenBackend {
    fn name(&self) -> &'static str {
        "js"
    }

    fn init(&self, sess: &Session) {
        use rustc_session::config::Lto;

        opts::init(sess);

        match sess.lto() {
            Lto::No | Lto::ThinLocal => {}
            Lto::Thin | Lto::Fat => {
                sess.dcx().fatal("LTO is not supported by rustc_codegen_mires");
            }
        }
    }

    fn thin_lto_supported(&self) -> bool {
        false
    }

    fn target_config(&self, sess: &Session) -> TargetConfig {
        // We never emit machine code, but rustc still checks that the target's ABI-required
        // features are enabled and warns otherwise, so report the mandated ones (cg_clif
        // src/lib.rs:156-172).
        let target_features = match sess.target.arch {
            Arch::X86_64 if sess.target.os != Os::None => {
                vec![sym::fxsr, sym::sse, sym::sse2, Symbol::intern("x87")]
            }
            Arch::AArch64 => match &sess.target.os {
                Os::None => vec![],
                Os::MacOs => vec![sym::neon, sym::aes, sym::sha2, sym::sha3],
                _ => vec![sym::neon],
            },
            // The backend's own target (scripts/compile.sh). WebAssembly has no ABI-required
            // features — `Target::abi_required_features` returns nothing for it — and every wasm
            // feature is a pure code generation choice we never make, so the set is empty.
            Arch::Wasm32 | Arch::Wasm64 => vec![],
            _ => vec![],
        };

        // `f16`/`f128` are out of scope for the spike.
        TargetConfig {
            unstable_target_features: target_features.clone(),
            target_features,
            has_reliable_f16: false,
            has_reliable_f16_math: false,
            has_reliable_f128: false,
            has_reliable_f128_math: false,
        }
    }

    fn target_cpu(&self, sess: &Session) -> String {
        match sess.opts.cg.target_cpu {
            Some(ref name) => name,
            None => sess.target.cpu.as_ref(),
        }
        .to_owned()
    }

    fn print_version(&self) {
        println!("rustc_codegen_mires version {}", env!("CARGO_PKG_VERSION"));
    }

    fn codegen_crate(&self, tcx: TyCtxt<'_>) -> Box<dyn Any> {
        if tcx.sess.codegen_units().as_usize() > 1 {
            // More than one CGU still codegens fine, but rustc refuses to copy several object
            // files onto a single `-o` path, so the .js output would go missing.
            tcx.dcx().warn(
                "rustc_codegen_mires expects `-Ccodegen-units=1`; with more than one codegen unit \
                 rustc will not copy the emitted JavaScript to the `-o` path",
            );
        }
        Box::new(rustc_codegen_ssa::base::codegen_crate(driver::JsDriver, tcx))
    }

    fn join_codegen(
        &self,
        ongoing_codegen: Box<dyn Any>,
        sess: &Session,
        outputs: &OutputFilenames,
        crate_info: &CrateInfo,
    ) -> (CompiledModules, WorkProductMap) {
        let (modules, work_products) = ongoing_codegen
            .downcast::<rustc_codegen_ssa::back::write::OngoingCodegen<driver::JsDriver>>()
            .unwrap()
            .join(sess, crate_info);

        // `--emit=obj` with no link step never reaches `CodegenBackend::link`, but the object it
        // leaves behind *is* the program `scripts/compile.sh` runs. Prune it here, so the one
        // crate case gets the same dead-code elimination and zombie reporting as a real link.
        if link::is_standalone_object(sess) {
            link::prune_objects(sess, outputs, &modules);
        }

        (modules, work_products)
    }

    fn link(
        &self,
        sess: &Session,
        compiled_modules: CompiledModules,
        crate_info: CrateInfo,
        metadata: EncodedMetadata,
        outputs: &OutputFilenames,
    ) {
        link::link(sess, compiled_modules, crate_info, metadata, outputs);
    }
}

/// Entry point for the hot-plugged backend (`-Zcodegen-backend=<this dylib>`).
#[unsafe(no_mangle)]
pub fn __rustc_codegen_backend() -> Box<dyn CodegenBackend> {
    Box::new(JsCodegenBackend)
}
