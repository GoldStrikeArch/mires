//! Allocation: the allocator shim rustc asks this backend to synthesize, and the zero value a
//! freshly zeroed heap block is filled with.
//!
//! # The shim
//!
//! A program that allocates calls `__rust_alloc`, `__rust_dealloc`, `__rust_realloc` and
//! `__rust_alloc_zeroed`. None of them has a body anywhere: `library/alloc` only *declares* them,
//! and whoever links the program is expected to supply the definitions. `rustc_codegen_ssa` asks
//! the backend for them through `ExtraBackendMethods::codegen_allocator`, handing over the list of
//! methods that still need one — [`rustc_ast::expand::allocator::ALLOCATOR_METHODS`] when the
//! program has no `#[global_allocator]`, and only the allocation error handler when it has.
//!
//! [`shim_items`] answers with real bodies over the runtime shim's heap: `__rt.alloc` and friends,
//! described in `CONTRACT.md`'s "Allocation" section. The shape is
//! `rustc_codegen_cranelift`'s `src/allocator.rs` — one function per method, named
//! `mangle_internal_symbol(global_fn_name(name))`, parameters in `method.inputs` order with a
//! `Layout` split into its size and its alignment — with one difference. cg_clif forwards every
//! method to the `default_fn_name` wrapper (`__rdl_alloc`) that `std` defines; there is no `std`
//! here, so the four methods rustc marks with a [`SpecialAllocatorMethod`] are answered directly
//! and only an unrecognized method still forwards.
//!
//! # The zero value
//!
//! A heap block is born byte granular and retyped at the first `*mut u8` to `*mut T` cast (see
//! `CONTRACT.md`). A block from `alloc_zeroed` has to come out of that retype holding `T`s worth
//! of zero rather than bytes of zero, and only the compiler knows what a zero `T` looks like:
//! `{ TAG: "None" }` for a niched `None`, `{ x: 0, y: 0 }` for a struct, `false` for a `bool`. So the
//! cast carries a **zero element factory** to `__rt.unscale`, and [`zero_factory`] is what builds
//! it.

use rustc_abi::Size;
use rustc_ast::expand::allocator::{
    ALLOC_ERROR_HANDLER, AllocatorMethod, AllocatorMethodInput, AllocatorTy,
    NO_ALLOC_SHIM_IS_UNSTABLE, SpecialAllocatorMethod, default_fn_name, global_fn_name,
};
use rustc_middle::mir::ConstValue;
use rustc_middle::mir::interpret::Allocation;
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::DUMMY_SP;
use rustc_symbol_mangling::mangle_internal_symbol;

use crate::cgu::CguCx;
use crate::item::{ItemKind, JsItem, JsName, Linkage, ZombieLog};
use crate::jsast::{self, Expr, Stmt};
use crate::value;

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
            vec![format!("{}$size", input.name), format!("{}$align", input.name)]
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
        items.push(shim_item(name, params, body, format!("`{}` allocator shim", method.name)));
    }

    // The marker `alloc::alloc::alloc` calls to make sure a program that allocates was linked with
    // a shim at all. It is a symbol whose existence is the whole of its meaning, so the body is
    // empty; cg_clif emits it unconditionally and so does this.
    items.push(shim_item(
        mangle_internal_symbol(tcx, NO_ALLOC_SHIM_IS_UNSTABLE),
        Vec::new(),
        Vec::new(),
        format!("`{NO_ALLOC_SHIM_IS_UNSTABLE}` allocator shim"),
    ));

    items
}

/// The third argument `__rt.unscale` takes: how to spell a zero element of the type being cast to.
///
/// `() => <zero>` when this backend can build one, and `null` when it cannot — in which case
/// `unscale` refuses a *zeroed* block of that type at run time, naming it, and a block that was
/// never zeroed is unaffected. The factory is an arrow rather than a value because it is called
/// once per element: a fresh object per element is what keeps two elements of an array of structs
/// from being one aliased object.
pub(crate) fn zero_factory<'tcx>(cgu: &CguCx<'tcx>, ty: Ty<'tcx>) -> Expr {
    match zero_value(cgu, ty) {
        Some(value) => jsast::arrow(Vec::new(), value),
        None => jsast::null(),
    }
}

/// The second argument `__rt.retype_rc` takes: how to spell the **header** of a struct with an
/// unsized tail.
///
/// The block such a struct allocates is laid out as the header record at element 0 and the tail
/// after it (`CONTRACT.md`, "A struct with an unsized tail"), so the reshape has to put an object
/// there for the field writes that follow to land in. `RcInner<[T]>` is `{ strong, weak, value }`
/// and the header is `{ strong, weak }`: every field but the tail, at its zero.
///
/// The values are zeros rather than anything meaningful because every one of them is written
/// immediately -- `try_allocate_for_layout` writes `strong` and `weak` before it hands the pointer
/// back. What the factory owes is the shape.
///
/// `null` when a field's zero cannot be spelled, in which case `retype_rc` refuses at run time and
/// names the type; an arrow rather than a value for the same reason [`zero_factory`] is one.
pub(crate) fn header_factory<'tcx>(cgu: &CguCx<'tcx>, ty: Ty<'tcx>) -> Expr {
    let tcx = cgu.tcx;
    let Some((tail, _)) = value::unsized_tail_indexed(tcx, ty) else { return jsast::null() };
    let ty = value::peel_pattern(value::peel_transparent(tcx, ty));
    let ty::Adt(def, args) = ty.kind() else { return jsast::null() };
    let variant = def.non_enum_variant();

    let mut entries = Vec::with_capacity(variant.fields.len());
    for (index, field) in variant.fields.iter_enumerated() {
        if index == tail {
            continue;
        }
        let field_ty = tcx.normalize_erasing_regions(value::typing_env(), field.ty(tcx, args));
        if value::is_zst(tcx, field_ty) {
            continue;
        }
        let Some(zero) = zero_value(cgu, field_ty) else { return jsast::null() };
        entries.push((crate::naming::Namer::field_name(variant, index), zero));
    }
    jsast::arrow(Vec::new(), jsast::object(entries))
}

/// A [zero factory](zero_factory) for a type whose zero is **not** the byte zero.
///
/// `None` where it is, and where this backend cannot spell one at all. A block from `alloc_zeroed`
/// already holds the number `0` in every byte, so a cast that needs no re-keying has nothing to do
/// unless the elements read back as something else: `vec![false; n]` is a block of `bool`, one byte
/// each, and `false` is not `0`.
pub(crate) fn non_byte_zero_factory<'tcx>(cgu: &CguCx<'tcx>, ty: Ty<'tcx>) -> Option<Expr> {
    let value = zero_value(cgu, ty)?;
    (jsast::expr_to_string(&value) != "0").then(|| jsast::arrow(Vec::new(), value))
}

/// The JavaScript value a `ty` whose bytes are all zero has, if it has one.
///
/// An all-zero allocation of the right size is decoded exactly the way a `static`'s initializer is,
/// so every shape the constant reader already knows — niches, transparent wrappers, nested structs,
/// arrays — is answered by construction rather than by a second table that could disagree with the
/// first. A type the reader cannot decode records a zombie; that is a question, not an answer, so
/// the probe log is thrown away and the caller is told `None`.
///
// TODO(stage3): unify with constant::raw — this is the same decode the constant reader does, and
// the two should share the entry point once `constant.rs` is free to change.
fn zero_value<'tcx>(cgu: &CguCx<'tcx>, ty: Ty<'tcx>) -> Option<Expr> {
    let tcx = cgu.tcx;
    let layout = tcx.layout_of(value::typing_env().as_query_input(ty)).ok()?;
    if !layout.is_sized() {
        return None;
    }
    let alloc = Allocation::from_bytes_byte_aligned_immutable(
        vec![0u8; layout.size.bytes_usize()],
        (),
    );
    let alloc_id = tcx.reserve_and_set_memory_alloc(tcx.mk_const_alloc(alloc));
    let probe = ZombieLog::default();
    let value = crate::constant::codegen_const_value(
        cgu,
        &probe,
        ConstValue::Indirect { alloc_id, offset: Size::ZERO },
        ty,
        DUMMY_SP,
    );
    probe.take().is_empty().then_some(value)
}
