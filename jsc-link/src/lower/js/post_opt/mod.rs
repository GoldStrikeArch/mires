//! Post-lowering JavaScript AST speed optimizations (`ReleaseSpeed`).
//!
//! Runs after `rcgjs_object::sroa::analyze_jir` and `rcgjs_object::optimizer::analyze` inside
//! [`super::function::finish`] so passes see SROA-exposed patterns (`[1][0]`, single-assign
//! literal temps like `k = "Some"` / `E = "children"` / `ah = 14`, `Reflect.get(a, "children")`,
//! and adjacent `$__mires_i64_div` rem+div pairs).

mod coalesce;
mod dce_copy;
mod float_opt;
mod fold;
mod guards;
mod shape;
mod switch_lut;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use rcgjs_object::jsast::{self, Expr, Stmt};

use super::ast_util::{collect_top_locals, tally_ident_reads_stmt};
use super::loop_opt;

pub(crate) use coalesce::coalesce_disjoint_locals_decl;
pub(super) use fold::{is_already_i32_expr, is_proven_nonneg_i31};

/// Entry point called from unit tests (runs all speed post-opts including code-duplicating
/// loop unrolling and loop-versioned BCE).
#[cfg(test)]
pub(super) fn run_speed_post_opts(decl: &mut Stmt) {
    let empty = HashSet::new();
    run_speed_post_opts_with_mode(decl, true, &empty);
}

/// Entry point called from `function::finish` on a lowered `Stmt::FunctionDecl`.
///
/// `allow_duplicating` is `false` during inline / ABI / representation cost-pricing trials so
/// code-duplicating speed optimizations (constant-trip loop unrolling and hot/cold loop-versioned
/// BCE) do not distort size budgets or block tiny inlines (`into_iter`, `read_be`) and flat-ABI
/// slice scalarization (`compress512`, `compress256`).
pub(super) fn run_speed_post_opts_with_mode(
    decl: &mut Stmt,
    allow_duplicating: bool,
    immutable_slices: &HashSet<String>,
) {
    let Stmt::FunctionDecl { params, body, .. } = decl else {
        return;
    };
    let mut local_names: HashSet<String> = params.iter().cloned().collect();
    let mut uninit_locals: HashSet<String> = HashSet::new();
    collect_top_locals(body, &mut local_names, &mut uninit_locals);

    // 1. Fold constant expressions, constant array indexing (`[1][0]`), single-assign literal
    //    propagation (`ah = 14`, `E = "children"`, `k = "Some"`), and constant switches.
    dce_copy::fold_constants_and_switches(body, &uninit_locals, &local_names);

    // 1b. Fast-path dead-`lo` `$__mires_i64_f64_to_bits(x)` sign-bit tests (`f64::is_sign_negative`
    //     in `f64::min`/`f64::max`), then re-fold any exposed constant branches (`c === 1`).
    if float_opt::fast_path_f64_sign_bit(body) {
        dce_copy::fold_constants_and_switches(body, &uninit_locals, &local_names);
    }

    // 1c. Fold IEEE-754 `Math.max` / `Math.min` labeled-block idioms (`d3-hierarchy`).
    if !local_names.contains("Math") {
        float_opt::fold_ieee_max_min(body);
    }

    // 2. Collapse nested `if (a) { if (b) { ... } }` and fold guarded `f2i` i32 clamps (`earcut`).
    loop_opt::fold_guarded_f2i(body);

    // 3. Elide redundant `.slice()` clones on single-use fresh array literals (`compress256`).
    loop_opt::elide_dead_fresh_slice_clones(body, &local_names);

    // 4. Unroll small pure constant-trip loops (`2..=10` iters, `sha3::permute`), then re-fold
    //    exposed constant arithmetic (`Math.imul`, `% 10`, `+ 1`) and dead bounds guards.
    if allow_duplicating && loop_opt::unroll_small_const_loops(body) {
        dce_copy::prop_straight_line_lits(body, &local_names);
        dce_copy::fold_constants_and_switches(body, &uninit_locals, &local_names);
    }

    // 5. Eliminate redundant dominated panic guards (`if (!aD)`, `if (!H)`, `if (bi)`) and
    //    constant-table / bitmask-proven bounds checks.
    loop_opt::eliminate_dominated_guards(body);

    // 6. Hoist loop-invariant `.buf`/`.off`/`.len` property loads across cold diverging panic
    //    calls (LICM), CSE repeated dominating `s.charCodeAt(idx)` loads, then version hot
    //    checked loops with a single exact pre-loop guard.
    let mut extra_decls: Vec<(String, Option<Expr>)> = Vec::new();
    dce_copy::cse_dominating_char_code_at(body, &mut local_names, &mut extra_decls);
    loop_opt::hoist_loop_invariant_props(
        body,
        &mut local_names,
        &mut extra_decls,
        immutable_slices,
    );
    if allow_duplicating {
        loop_opt::version_checked_loops(body);
    }

    // 6b. Fold proven-integer two-sided range checks `(c >= A && c <= B)` -> `(c - A >>> 0) <= (B - A)`
    //     (including `if (A <= c) { b = c <= B; } else { b = false; }` and negated `||` forms).
    //     Runs before `smi_ize_counted_loop_steps` so `m = m + 1 >>> 0` retains its exact `U32`
    //     non-negativity proof for in-bounds `s.charCodeAt(m)` recognition.
    guards::fold_unsigned_range_checks(body, &local_names);

    // 6c. Lower dense ASCII delimiter membership (`>= 6` byte constants in `0..=255` tested against
    //     a proven-numeric value, in `||`/`&&` chains or 2-arm `switch` statements) into a 128- or
    //     256-byte `Uint8Array` LUT (hoisted & deduplicated to module scope by `hoist_consts`).
    //     Gated by `allow_duplicating` so inline/ABI pricing trials never count a 128-element array.
    if allow_duplicating && !local_names.contains("Uint8Array") {
        switch_lut::lower_dense_ascii_luts(body);
    }

    // 6d. Lower chained string/integer equality `if / else if` and labeled guard chains (`>= 4`
    //     arms on the same pure identifier) into `switch (x)`.
    switch_lut::lower_chained_eq_to_switch(body);

    // 6e. Smi-ize counted loop steps (`i = i + 1 >>> 0` -> `i = i + 1 | 0`) and demote
    //     bit-exact 32-bit loop-carried registers (`x + z >>> 0` -> `x + z | 0`, `compress256`).
    loop_opt::smi_ize_counted_loop_steps(body);
    loop_opt::demote_bitexact_u32_wraps(body, params);

    // 6f. Fold Rust `ToInt32` labeled-block diamonds (`earcut` `zorder` in `earcut_linked` and
    //     `is_ear_hashed_sl`) to `dst = src | 0` (or `dst = src >>> 0`) when all intermediate
    //     temporaries written in the slow path are dead after the block.
    if !local_names.contains("Math") {
        shape::fold_to_int32_diamonds(body, &local_names);
    }

    // 7. Spec-identical `Reflect.get(o, k)` fast-path + dominated object-guard CSE.
    if !local_names.contains("Reflect") {
        dce_copy::optimize_reflect_get(body);
    }

    // 8. Fuse adjacent `$__mires_i64_div(..., rem) + $__mires_i64_div(..., div)` pairs on
    //    identical unchanged operands into a single `$__mires_i64_divrem` call.
    shape::fuse_i64_div_rem(body, &mut local_names, &mut extra_decls);

    if !extra_decls.is_empty() {
        prepend_decls(body, extra_decls);
    }
}

fn prepend_decls(body: &mut Vec<Stmt>, extra: Vec<(String, Option<Expr>)>) {
    if let Some(Stmt::Lets(existing)) = body.first_mut() {
        existing.extend(extra);
    } else {
        body.insert(0, jsast::lets(extra));
    }
}

pub(super) fn alloc_fresh_local(
    local_names: &mut HashSet<String>,
    extra_decls: &mut Vec<(String, Option<Expr>)>,
    prefix: &str,
) -> String {
    let mut n = 0usize;
    loop {
        let name = format!("{prefix}{n}");
        if local_names.insert(name.clone()) {
            extra_decls.push((name.clone(), None));
            return name;
        }
        n += 1;
    }
}

pub(super) fn tally_ident_reads(stmts: &[Stmt]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for stmt in stmts {
        tally_ident_reads_stmt(stmt, &mut counts);
    }
    counts
}
