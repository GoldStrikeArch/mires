//! Allocation: the allocator shim rustc asks this backend to synthesize.
//!
//! A program that allocates calls `__rust_alloc`, `__rust_dealloc`, `__rust_realloc` and
//! `__rust_alloc_zeroed`. None of them has a body anywhere: `library/alloc` only *declares* them,
//! and whoever links the program is expected to supply the definitions. `rustc_codegen_ssa` asks
//! the backend for them through `ExtraBackendMethods::codegen_allocator`, handing over the list of
//! methods that still need one — [`rustc_ast::expand::allocator::ALLOCATOR_METHODS`] when the
//! program has no `#[global_allocator]`, and only the allocation error handler when it has.

use rustc_ast::expand::allocator::{
    default_fn_name, global_fn_name, AllocatorKind, AllocatorMethod, AllocatorMethodInput,
    AllocatorTy, SpecialAllocatorMethod, ALLOC_ERROR_HANDLER, NO_ALLOC_SHIM_IS_UNSTABLE,
};
use rustc_middle::ty::TyCtxt;
use rustc_symbol_mangling::mangle_internal_symbol;

use crate::item::{AllocatorProvider, ItemKind, JsItem, JsName, Linkage};
use crate::jsast::{self, Expr, Stmt};

/// The `__rt` member each of the four known methods is answered with.
fn rt_member(special: SpecialAllocatorMethod) -> &'static str {
    match special {
        SpecialAllocatorMethod::Alloc => "alloc",
        SpecialAllocatorMethod::AllocZeroed => "alloc_zeroed",
        SpecialAllocatorMethod::Dealloc => "dealloc",
        SpecialAllocatorMethod::Realloc => "realloc",
    }
}

/// The JavaScript parameter names one input contributes, in order.
///
/// A `Layout` is two machine values rather than one — rustc splits it into a size and an alignment
/// at the ABI boundary, and the declaration in `library/alloc/src/alloc.rs` spells them out — so it
/// contributes two parameters. They are named after the input with a `$` suffix, the same way a
/// splatted `"rust-call"` tuple parameter is (`naming.rs`), which keeps them from ever colliding
/// with an input a future method list happens to call `size`.
fn input_params(input: &AllocatorMethodInput) -> Vec<String> {
    match input.ty {
        AllocatorTy::Layout => {
            vec![
                format!("{}$size", input.name),
                format!("{}$align", input.name),
            ]
        }
        _ => vec![input.name.to_string()],
    }
}

/// The alignment a `Layout` parameter carries, as a plain number.
///
/// `Layout::align` is a `core::mem::Alignment`, which is a `repr(transparent)` struct over a
/// fieldless `repr(usize)` enum, and such an enum *is* its discriminant in this model
/// (`value::EnumRepr`). So the parameter already holds the number the shim wants, and there is
/// nothing to read out of it.
fn alignment(param: &str) -> Expr {
    jsast::id(param)
}

/// The body of one shim method: the arguments it forwards, in the order `__rt` takes them.
///
/// Every `__rt` allocation member takes `(size, align)` where the Rust method takes a `Layout`, and
/// the remaining inputs ride along in their declared positions. `dealloc` returns nothing; the rest
/// return what they are handed.
fn special_body(method: &AllocatorMethod, special: SpecialAllocatorMethod) -> Vec<Stmt> {
    let mut args = Vec::new();
    for input in method.inputs {
        match input.ty {
            AllocatorTy::Layout => {
                args.push(jsast::id(format!("{}$size", input.name)));
                args.push(alignment(&format!("{}$align", input.name)));
            }
            _ => args.push(jsast::id(input.name)),
        }
    }
    let call = jsast::rt_call(rt_member(special), args);
    match method.output {
        AllocatorTy::ResultPtr => vec![jsast::ret(call)],
        _ => vec![jsast::expr_stmt(call)],
    }
}

/// The body of the allocation error handler: a diverging call into the shim.
///
/// `handle_alloc_error` is `-> !`, so there is nothing to return and nothing after the call;
/// `__rt.alloc_error` throws.
fn alloc_error_body(method: &AllocatorMethod) -> Vec<Stmt> {
    let mut args = Vec::new();
    for input in method.inputs {
        match input.ty {
            AllocatorTy::Layout => {
                args.push(jsast::id(format!("{}$size", input.name)));
                args.push(alignment(&format!("{}$align", input.name)));
            }
            _ => args.push(jsast::id(input.name)),
        }
    }
    vec![jsast::expr_stmt(jsast::rt_call("alloc_error", args))]
}

/// The body cg_clif gives every method: forward to the `default_fn_name` wrapper.
///
/// Nothing in a `core` + `alloc` sysroot defines one, so this is reached only if a future method
/// list grows an entry this module has not been taught about. Forwarding keeps that case a missing
/// item at link time, which names the symbol, rather than a silently wrong answer.
fn forwarding_body(tcx: TyCtxt<'_>, method: &AllocatorMethod, params: &[String]) -> Vec<Stmt> {
    let callee = mangle_internal_symbol(tcx, &default_fn_name(method.name));
    let args = params.iter().map(jsast::id).collect();
    let call = jsast::call(jsast::id(callee), args);
    match method.output {
        AllocatorTy::ResultPtr => vec![jsast::ret(call)],
        _ => vec![jsast::expr_stmt(call)],
    }
}

/// One allocator shim function, under the exact symbol the declarations in `library/alloc` are
/// compiled to call.
///
/// The linkage keeps that name through minification and stops there: an allocator shim is not a
/// dead code elimination root, so a program that never allocates does not carry one, and it is no
/// part of the program's interface either.
fn shim_item(name: String, params: Vec<String>, body: Vec<Stmt>, debug_path: String) -> JsItem {
    JsItem::new(
        JsName::new(name.clone()),
        ItemKind::Alloc,
        jsast::function(name.as_str(), params, body),
        Vec::new(),
        Linkage::fixed(name),
        debug_path,
    )
}

/// Every item the allocator shim consists of, for the methods rustc says still need one.
pub(crate) fn shim_items(tcx: TyCtxt<'_>, methods: &[AllocatorMethod]) -> Vec<JsItem> {
    let mut items = Vec::with_capacity(methods.len() + 1);

    for method in methods {
        let name = mangle_internal_symbol(tcx, &global_fn_name(method.name));
        let params: Vec<String> = method.inputs.iter().flat_map(input_params).collect();
        let body = match method.special {
            Some(special) => special_body(method, special),
            None if method.name == ALLOC_ERROR_HANDLER => alloc_error_body(method),
            None => forwarding_body(tcx, method, &params),
        };
        items.push(shim_item(
            name,
            params,
            body,
            format!("`{}` allocator shim", method.name),
        ));
    }

    // The marker `alloc::alloc::alloc` calls to make sure a program that allocates was linked with
    // a shim at all. It is a symbol whose existence is the whole of its meaning, so the body is
    // empty; cg_clif emits it unconditionally and so does this.
    let provider = allocator_provider(tcx.allocator_kind(()));
    items.push(
        shim_item(
            mangle_internal_symbol(tcx, NO_ALLOC_SHIM_IS_UNSTABLE),
            Vec::new(),
            Vec::new(),
            format!("`{NO_ALLOC_SHIM_IS_UNSTABLE}` allocator shim"),
        )
        .with_allocator_provider(provider),
    );

    items
}

fn allocator_provider(kind: Option<AllocatorKind>) -> AllocatorProvider {
    match kind {
        Some(AllocatorKind::Default) => AllocatorProvider::Default,
        Some(AllocatorKind::Global) => AllocatorProvider::Custom,
        None => AllocatorProvider::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rustc_allocator_kinds_map_to_conservative_artifact_facts() {
        assert_eq!(
            allocator_provider(Some(AllocatorKind::Default)),
            AllocatorProvider::Default
        );
        assert_eq!(
            allocator_provider(Some(AllocatorKind::Global)),
            AllocatorProvider::Custom
        );
        assert_eq!(allocator_provider(None), AllocatorProvider::Unknown);
    }
}
