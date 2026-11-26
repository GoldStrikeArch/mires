//! Post-lowering JavaScript AST optimization passes invoked from [`super::function`].
//!
//! Keeps function lowering (`function.rs`) focused on ABI setup, region emission, and item
//! finalization while organizing the post-lowering JS AST cleanup passes by concern:
//! - [`slot_fold`]: offset-slot deref folding (`fold_offset_slot_derefs`) and deref-only
//!   `__rt.sfp` elimination (`fold_sfp_deref_only`).
//! - [`slice_cse`]: zero-offset index simplification (`fold_zero_offset_arith`) and repeated
//!   slice-load CSE (`cse_slice_loads`).
//! - [`misc`]: dead local write elimination, ephemeral `__rt.box` collapse, single-slot array
//!   unboxing, and `bytes_str` / `bstr_in` local fusion.

mod misc;
mod slice_cse;
mod slot_fold;

use std::collections::HashSet;

use rcgjs_object::jsast::{Expr, Stmt};

#[cfg(test)]
pub(super) use misc::fuse_local_bytes_str;
pub(super) use misc::fuse_local_bytes_str_with;
pub(crate) use misc::{fold_bstr_in_bytes_decl, fold_bstr_in_bytes_stmts};
pub(super) use slice_cse::{cse_slice_loads, fold_zero_offset_arith};
pub(super) use slot_fold::{fold_offset_slot_derefs, fold_sfp_deref_only};

/// Configuration flags controlling which local post-lowering AST passes run in
/// [`super::function::lower_with_ascii`].
pub(super) struct LocalOptConfig {
    pub ast_dce: bool,
    pub needs_byte_slice_ref: bool,
    pub bstr_enabled: bool,
    pub cse_slice_loads: bool,
}

/// Runs the pre-`finish` local post-lowering passes on a lowered function body, preserving the
/// exact pass ordering expected by downstream SROA and speed optimizations.
pub(super) fn run_local_passes(
    body: &mut Vec<Stmt>,
    declarations: &mut Vec<(String, Option<Expr>)>,
    local_names: &mut HashSet<String>,
    cfg: LocalOptConfig,
) {
    if cfg.ast_dce {
        let uninit_locals: HashSet<String> = declarations
            .iter()
            .filter(|(_, init)| init.is_none())
            .map(|(name, _)| name.clone())
            .collect();
        misc::collapse_ephemeral_rt_box_deref(body, &uninit_locals);
        misc::eliminate_dead_local_writes(body, local_names);
        misc::fuse_local_bytes_str_with(body, &uninit_locals, cfg.needs_byte_slice_ref);
        slice_cse::fold_zero_offset_arith(body, &uninit_locals);
        slot_fold::fold_offset_slot_derefs(body, &uninit_locals);
        misc::eliminate_dead_local_writes(body, local_names);
        slot_fold::fold_sfp_deref_only(body, &uninit_locals);
        if cfg.bstr_enabled {
            misc::fuse_local_bstr_in_bytes(body, &uninit_locals);
        }
        if cfg.cse_slice_loads {
            slice_cse::cse_slice_loads(body, declarations, local_names);
        }
        misc::eliminate_dead_local_writes(body, local_names);
        misc::unbox_single_slot_array_locals(body, declarations);
    }
    if cfg.bstr_enabled {
        misc::fold_bstr_in_bytes_stmts(body);
    }
}

/// Runs post-SROA slot/offset/dead-write cleanup passes inside [`super::function::finish`].
pub(super) fn run_post_sroa_passes(params: &[String], body: &mut Vec<Stmt>) {
    let (local_names, uninit_locals) = collect_fn_locals(params, body);
    slice_cse::fold_zero_offset_arith(body, &uninit_locals);
    slot_fold::fold_offset_slot_derefs(body, &uninit_locals);
    misc::eliminate_dead_local_writes(body, &local_names);
    slot_fold::fold_sfp_deref_only(body, &uninit_locals);
    misc::eliminate_dead_local_writes(body, &local_names);
}

/// Runs final zero-offset and dead-write cleanup after `ReleaseSpeed` post-optimizations inside
/// [`super::function::finish`].
pub(super) fn run_post_speed_cleanup(params: &[String], body: &mut Vec<Stmt>) {
    let (local_names, uninit_locals) = collect_fn_locals(params, body);
    slice_cse::fold_zero_offset_arith(body, &uninit_locals);
    misc::eliminate_dead_local_writes(body, &local_names);
}

fn collect_fn_locals(params: &[String], body: &[Stmt]) -> (HashSet<String>, HashSet<String>) {
    let mut local_names: HashSet<String> = params.iter().cloned().collect();
    let mut uninit_locals: HashSet<String> = HashSet::new();
    super::ast_util::collect_top_locals(body, &mut local_names, &mut uninit_locals);
    (local_names, uninit_locals)
}
