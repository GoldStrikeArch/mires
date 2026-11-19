//! Host calls: `#[js_extern]` declarations, `js!{}` blocks, and the `rcgjs.host.*`/`rcgjs.tc.*`/
//! `rcgjs.map.*` marker functions (v1 `backend/src/host.rs`, `js_extern.rs`, `js_block.rs`,
//! `map.rs`).
//!
//! # The seam (W8 → W9)
//!
//! `call.rs` owns the Rust calling convention and routes everything host-shaped here **before**
//! it looks at the callee, so this module sees the call exactly as the encoder recorded it:
//!
//! - every `OpKind::Call` whose `host` is `Some(payload)` (that is every `CallSemantic::{Host,
//!   ViewAbi, RawJavaScript}` call and every marker call), and
//! - every `OpKind::Call` whose `Callee::Direct` link name is a `rcgjs.host.*`/`rcgjs.tc.*`/
//!   `rcgjs.map.*` marker that arrived without a payload,
//!
//! through [`lower_call`]; and the payload-only op `OpKind::Host` through [`lower_host`].
//!
//! Both take the whole [`Op`] rather than its pieces: a host lowering needs the argument values
//! (to spell them, and to look through their defining ops for the `&STATIC` a `rcgjs.tc.template`
//! marker takes), the result value (to know the result type), and the origin. Everything else is
//! reached through [`JsCx`] as in every other lowering module. The return value is the
//! **expression** of the call, as `call.rs` returns: `emit.rs` binds it to the result's name, or
//! emits it as an expression statement when the result is unused. A host lowering that has to
//! emit statements hoists them with `JsCx::temp` (as `ptr.rs` does) and returns the temporary.
//!
//! Every unsupported shape is a [`decline`], never a panic.
//!
//! # What a link-time host lowering cannot do yet
//!
//! # What v1 asked `tcx` and this module reads off the object instead
//!
//! * a **closure's body**, by the [`ItemId`] its type spells (`type_id_hash(instance.ty)` of a
//!   closure instance IS the closure type), through [`super::super::LowerInput::items`];
//! * the **drop glue** of a closure or a future a marker takes ownership of, through
//!   [`super::super::LowerInput::drop_glue`] (a body says it is one: `BodySemantic::DropGlue`). A
//!   type whose `LayoutFacts::needs_drop` is false has empty glue and needs no callback;
//! * a marker's **sibling helper instances** — `poll_once::<F>`/`poll_result::<F>` for the async
//!   markers, `display_to_str::<T>` for `rcgjs.tc.text` — off `HostPayload::helper`.

use std::collections::{BTreeMap, BTreeSet};

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::repr::{Op, OpKind, ValueId};
use rcgjs_object::jir::semantic::{
    Callee, DefinitionId, HostPayload, ItemId, KeyedListPayload, TypeId,
};
use rcgjs_object::jir::types::TypeKind;
use rcgjs_object::jsast::{self, BinOp, Expr, UnOp};
use rcgjs_object::ptr;

use crate::block::{self, Block};
use crate::descriptor::{Descriptor, Root, Shape};

use super::{decline, value, JsCx, Res};

/// The `link_section` prefix every host marker carries (v1 `host.rs::HOST_PREFIX`).
pub const HOST_PREFIX: &str = "rcgjs.host.";
/// The `link_section` prefix every `view-abi` marker carries (v1 `abi.rs::MARKER_PREFIX`).
pub const VIEW_ABI_PREFIX: &str = "rcgjs.tc.";
/// The `link_section` prefix every `mires-js` collection marker carries (v1 `map.rs`).
pub const MAP_PREFIX: &str = "rcgjs.map.";

/// The parameter that gives a loop-created thunk its own closure environment.
const CAPTURED_ENVIRONMENT: &str = "$c";

/// Whether a host marker has a safe Rust body that optimizer modes may replace
/// (v1 `host.rs::is_async_marker`).
pub fn is_async_marker(marker: &str) -> bool {
    matches!(
        marker,
        "spawn" | "spawnowner" | "spawnowner.v2" | "futurepromise" | "futurenativepromise"
    )
}

/// Whether a `Callee::Direct` link name is one of the marker functions W9 intercepts.
///
/// The encoder records a payload for every marker it recognizes; this is the belt-and-braces
/// check `call.rs` applies to a `Direct` callee that arrived without one, so a marker never
/// lowers to a call to the marker's own (empty) body.
#[allow(dead_code)]
pub fn is_marker_name(name: &str) -> bool {
    name.starts_with(HOST_PREFIX)
        || name.starts_with(VIEW_ABI_PREFIX)
        || name.starts_with(MAP_PREFIX)
}

/// What one host call turned out to be, after the payload has been read.
enum Family<'a> {
    Extern(&'a [u8]),
    Block(&'a [u8]),
    Host(String),
    ViewAbi(String),
    Map(String),
    KeyedList(&'a KeyedListPayload),
}

/// The payload family of a call, or `None` for a call this module does not own.
fn family<'a>(cx: &'a JsCx<'_>, op: &'a Op) -> Option<Family<'a>> {
    let OpKind::Call { callee, host, .. } = &op.kind else {
        return None;
    };
    match host {
        Some(HostPayload::Extern(bytes)) => cx.interns.bytes(*bytes).map(Family::Extern),
        Some(HostPayload::Block(bytes)) => cx.interns.bytes(*bytes).map(Family::Block),
        Some(HostPayload::Marker { section, .. }) => {
            let section = cx.interns.str(*section)?;
            if let Some(marker) = section.strip_prefix(HOST_PREFIX) {
                return Some(Family::Host(marker.to_owned()));
            }
            section
                .strip_prefix(VIEW_ABI_PREFIX)
                .map(|marker| Family::ViewAbi(marker.to_owned()))
        }
        Some(HostPayload::ViewAbi { marker, .. }) => cx
            .interns
            .str(*marker)
            .map(|m| Family::ViewAbi(m.to_owned())),
        Some(HostPayload::Map { marker }) => {
            cx.interns.str(*marker).map(|m| Family::Map(m.to_owned()))
        }
        Some(HostPayload::KeyedList(payload)) => Some(Family::KeyedList(payload)),
        // A marker the encoder did not recognize, reaching here through its link name alone.
        None => {
            let Callee::Direct { link_name, .. } = callee else {
                return None;
            };
            let name = cx.interns.str(*link_name)?;
            if let Some(marker) = name.strip_prefix(HOST_PREFIX) {
                return Some(Family::Host(marker.to_owned()));
            }
            if let Some(marker) = name.strip_prefix(VIEW_ABI_PREFIX) {
                return Some(Family::ViewAbi(marker.to_owned()));
            }
            name.strip_prefix(MAP_PREFIX)
                .map(|marker| Family::Map(marker.to_owned()))
        }
    }
}

/// Whether this call is one the JavaScript lowering intercepts instead of calling the callee's
/// body.
///
/// Every payload family is intercepted, with one exception v1 makes too: the async markers
/// (`spawn`/`spawnowner`/`futurepromise`) have safe Rust bodies of their own and are compiled as
/// ordinary calls unless the optimizer's future-poller pass applies to this compilation
/// (v1 `abi.rs::codegen_call`).
pub fn intercepts(cx: &JsCx<'_>, op: &Op) -> bool {
    match family(cx, op) {
        Some(Family::Host(marker)) if is_async_marker(&marker) => cx
            .opts
            .applies(rcgjs_object::opts::OptimizerPass::FuturePoller),
        Some(_) => true,
        None => false,
    }
}

/// Lowers a call the JavaScript lowering intercepts instead of calling the callee's body.
///
/// `op.kind` is an `OpKind::Call`; its `host` is the payload when the encoder recorded one.
pub fn lower_call(cx: &JsCx<'_>, op: &Op) -> Res<Expr> {
    let OpKind::Call { args, .. } = &op.kind else {
        return Err(decline(OpClass::Host, "not a call"));
    };
    let Some(family) = family(cx, op) else {
        return Err(decline(
            OpClass::Host,
            "a call with a host payload this lowering does not recognize",
        ));
    };
    match family {
        Family::Extern(bytes) => js_extern(cx, op, bytes, args),
        Family::Block(bytes) => js_block(cx, op, bytes, args),
        Family::Host(marker) => host_marker(cx, op, &marker, args),
        Family::ViewAbi(marker) => super::template::lower_marker(cx, op, &marker, args),
        Family::Map(marker) => map_marker(cx, op, &marker, args),
        Family::KeyedList(payload) => super::template::lower_keyed_list(cx, payload, args),
    }
}

/// Lowers a fixed argument-array call after its private-storage proof.
pub fn lower_host(cx: &JsCx<'_>, op: &Op) -> Res<Expr> {
    let invalid = || decline(OpClass::Host, "unsupported payload-only host operation");
    let OpKind::Host {
        payload: HostPayload::Marker { section, .. },
        args,
    } = &op.kind
    else {
        return Err(invalid());
    };
    let operation = cx
        .interns
        .str(*section)
        .and_then(|section| section.strip_prefix(HOST_PREFIX))
        .and_then(rcgjs_object::host::HostOperation::from_marker)
        .ok_or_else(invalid)?;
    let array = args.last().ok_or_else(invalid)?;
    let Some(TypeKind::Array { length, .. }) = cx.table.get(cx.ty_of(*array)?).map(|ty| &ty.kind)
    else {
        return Err(invalid());
    };
    let mut arguments = args
        .iter()
        .map(|arg| cx.expr_of(*arg))
        .collect::<Res<Vec<_>>>()?;
    if *length == 0 {
        // Rust's zero-sized array carrier is erased. Reflect still requires a fresh object.
        let empty = Expr::Array(Vec::new());
        if let Some(result) = op.result {
            cx.record_allocation_origin(result, &empty);
        }
        *arguments.last_mut().unwrap() = empty;
    }
    operation
        .lower_array(arguments)
        .map_err(|detail| decline(OpClass::Host, detail))
}

// ---------------------------------------------------------------------------------------------
// `rcgjs.host.*`
// ---------------------------------------------------------------------------------------------

/// Lowers one `mires-js` host marker (v1 `host.rs::codegen_host_marker`).
fn host_marker(cx: &JsCx<'_>, op: &Op, marker: &str, args: &[ValueId]) -> Res<Expr> {
    // The async markers keep the ordinary safe Rust bodies they have unless the optimizer's
    // future-poller pass applies, exactly as v1's `abi.rs::codegen_call` decides; `intercepts`
    // is what gates that, so reaching here means the pass applies.
    if is_async_marker(marker) {
        return future_driver(cx, op, marker, args);
    }

    let wrong = |what: &str| {
        Err(decline(
            OpClass::Host,
            format!("`{HOST_PREFIX}{marker}` {what}"),
        ))
    };
    let values = |cx: &JsCx<'_>| -> Res<Vec<Expr>> {
        args.iter()
            .map(|arg| host_argument(cx, *arg, cx.expr_of(*arg)?))
            .collect()
    };

    // The markers that read a closure rather than a value come first: their argument is a Rust
    // closure environment and what the host is handed is a thunk that calls its body.
    match marker {
        "callback0" | "callback1" | "callback2" | "callback3" | "callback4" | "promise"
        | "localwaker" | "microtask" | "task" => {
            let [closure] = args else {
                return wrong("takes one closure");
            };
            let (setup, arrow, _) = closure_thunk(cx, *closure)?;
            let value = match marker {
                "promise" => jsast::new(jsast::id("Promise"), vec![arrow]),
                "localwaker" => jsast::rt_call("host_local_waker", vec![arrow]),
                "microtask" => jsast::rt_call("microtask", vec![arrow]),
                "task" => jsast::rt_call("host_task", vec![arrow]),
                _ => arrow,
            };
            return Ok(super::ptr::sequenced(setup, value));
        }
        "try" => {
            let [closure] = args else {
                return wrong("takes one closure");
            };
            let (setup, arrow, _) = closure_thunk(cx, *closure)?;
            let mut arguments = vec![arrow];
            arguments.extend(closure_drop(cx, *closure)?);
            return Ok(super::ptr::sequenced(
                setup,
                jsast::rt_call("host_try", arguments),
            ));
        }
        "settled" => {
            let [value, closure] = args else {
                return wrong("takes a value and a closure");
            };
            let value = cx.expr_of(*value)?;
            let (setup, arrow, _) = closure_thunk(cx, *closure)?;
            return Ok(super::ptr::sequenced(
                setup,
                jsast::rt_call("settled", vec![value, arrow]),
            ));
        }
        _ => {}
    }

    let values = values(cx)?;
    if let Some(operation) = rcgjs_object::host::HostOperation::from_marker(marker) {
        return operation
            .lower(values)
            .map_err(|error| decline(OpClass::Host, error));
    }
    let one = || -> Res<Expr> {
        values.first().cloned().ok_or_else(|| {
            decline(
                OpClass::Host,
                format!("`{HOST_PREFIX}{marker}` takes one argument"),
            )
        })
    };
    match marker {
        "iserror" => Ok(jsast::rt_call("host_is_error", vec![one()?])),
        "base64url" => Ok(jsast::rt_call("host_base64url", vec![one()?])),
        "base64urlnative" => Ok(jsast::method_call(
            one()?,
            "toBase64",
            vec![jsast::object(vec![
                ("alphabet".to_owned(), jsast::string("base64url")),
                ("omitPadding".to_owned(), jsast::boolean(true)),
            ])],
        )),
        "base64urlsupported" => {
            if !values.is_empty() {
                return wrong("takes no arguments");
            }
            Ok(jsast::binary(
                BinOp::StrictEq,
                jsast::unary(
                    UnOp::TypeOf,
                    jsast::member(
                        jsast::member(jsast::id("Uint8Array"), "prototype"),
                        "toBase64",
                    ),
                ),
                jsast::string("function"),
            ))
        }
        "badread" => {
            if !values.is_empty() {
                return wrong("takes no arguments");
            }
            Ok(throw_expr(jsast::new(
                jsast::id("TypeError"),
                vec![jsast::string(
                    "host value does not match its Rust representation",
                )],
            )))
        }
        // A JavaScript Number has f64 precision. Reading one as Rust `f32` must round at the
        // boundary, before a comparison or move can observe extra precision.
        "read" => {
            let value = one()?;
            let Some(result) = op.result else {
                return Ok(value);
            };
            boundary_value(cx, cx.ty_of(result)?, value)
        }
        "throw" => Ok(throw_expr(one()?)),
        other => Err(decline(
            OpClass::Host,
            format!("`{HOST_PREFIX}{other}` is not a host marker this backend knows"),
        )),
    }
}

/// `throw value` as an expression: the statement wrapped in an immediately called arrow.
///
/// v1 emitted a `throw` statement, which the seam's expression result cannot carry. The two are
/// the same control flow — the call never returns — and the wrapper is what makes it spellable in
/// an operand position.
fn throw_expr(value: Expr) -> Expr {
    jsast::call(
        jsast::arrow_block(Vec::new(), vec![jsast::throw(value)]),
        Vec::new(),
    )
}

// ---------------------------------------------------------------------------------------------
// Closures
// ---------------------------------------------------------------------------------------------

/// The thunk a marker is handed for a closure argument, and the call inside it (v1
/// `abi.rs::closure_thunk`).
///
/// Answers the optional assignment that materializes an inline environment, the arrow
/// `(...$aN) => body(env, ...$aN)`, and the call `body(env, ...$aN)` alone.
///
/// v1 resolved the closure's own instance through `tcx`; here the closure type's `DefinitionId`
/// and the type's own id are the instance's [`ItemId`] (the encoder builds one as
/// `type_id_hash(instance.ty)`, which for a closure instance is the closure type), and the item
/// index answers with its JavaScript name and its parameter count.
///
/// # The arrow snapshots its environment inside a loop
///
/// An environment is a value the marker takes ownership of at the call. Two cases need care:
///
/// * an **inline** environment (`JsCx::expr_of` spells an inlined value at its use site) is first
///   assigned to a generated temporary, so the host cannot rebuild it when it runs the thunk — the
///   `Rc::clone` of an already scheduled task, run after the task had moved on;
/// * a **loop-reused** binding would be read again on a later turn, so every row's handler would
///   see the last row's environment (v1's `in_cycle` snapshot, `abi.rs::closure_thunk`). A hoisted
///   temporary alone does not help because the loop reuses that binding too. The arrow therefore
///   receives its current value through an immediately invoked wrapper.
///
/// A bare identifier in a body with no cycle is neither, and is spelled as v1 spelled it. The
/// call form runs in the marker's own effect or accessor and matches v1's direct spelling.
pub(super) fn closure_thunk(cx: &JsCx<'_>, closure: ValueId) -> Res<(Option<Expr>, Expr, Expr)> {
    closure_thunk_with(cx, closure, true, true)
}

pub(super) fn closure_thunk_with(
    cx: &JsCx<'_>,
    closure: ValueId,
    convert_params_in: bool,
    convert_result_out: bool,
) -> Res<(Option<Expr>, Expr, Expr)> {
    let ty = cx.ty_of(closure)?;
    let (name, arity, param_tys, result_ty, has_environment) =
        closure_item(cx, ty, convert_params_in, convert_result_out)?;
    let params: Vec<String> = (0..arity).map(|index| format!("$a{index}")).collect();
    let (setup, environment) = match has_environment {
        true => {
            let value = cx.expr_of(closure)?;
            match value {
                Expr::Ident(_) => (None, Some(value)),
                _ => {
                    let (_, held) = cx.temp(value.clone());
                    (Some(jsast::assign(held.clone(), value)), Some(held))
                }
            }
        }
        false => (None, None),
    };

    let convert_params = || -> Res<Vec<Expr>> {
        if !crate::lower::bstr::enabled() || !convert_params_in {
            return Ok(params.iter().cloned().map(jsast::id).collect());
        }
        params
            .iter()
            .zip(&param_tys)
            .map(|(param, param_ty)| {
                crossing(
                    cx,
                    *param_ty,
                    jsast::id(param.clone()),
                    crate::lower::bstr::Crossing::In,
                )
            })
            .collect()
    };
    let convert_result = |call_expr: Expr| -> Res<Expr> {
        if !crate::lower::bstr::enabled() || !convert_result_out {
            return Ok(call_expr);
        }
        match result_ty {
            Some(res_ty) => crossing(cx, res_ty, call_expr, crate::lower::bstr::Crossing::Out),
            None => Ok(call_expr),
        }
    };

    let mut raw_arguments = Vec::with_capacity(arity + 1);
    raw_arguments.extend(environment.clone());
    raw_arguments.extend(params.iter().cloned().map(jsast::id));
    let call = convert_result(jsast::call(jsast::id(name.clone()), raw_arguments))?;

    let arrow = match environment.clone().filter(|_| function_has_cycle(cx)) {
        Some(environment) => {
            let mut arguments = Vec::with_capacity(arity + 1);
            arguments.push(jsast::id(CAPTURED_ENVIRONMENT));
            arguments.extend(convert_params()?);
            let inner_call = convert_result(jsast::call(jsast::id(name), arguments))?;
            let inner = jsast::arrow(params, inner_call);
            jsast::call(
                jsast::arrow(vec![CAPTURED_ENVIRONMENT.to_owned()], inner),
                vec![environment],
            )
        }
        None => {
            let mut arguments = Vec::with_capacity(arity + 1);
            arguments.extend(environment);
            arguments.extend(convert_params()?);
            let arrow_call = convert_result(jsast::call(jsast::id(name), arguments))?;
            jsast::arrow(params, arrow_call)
        }
    };
    Ok((setup, arrow, call))
}

/// Whether this function's control flow graph has a cycle at all.
///
/// v1 asked whether the *block* the thunk is built in is in a cycle (`base.rs::in_cycle`), which
/// the JavaScript side of a link-time lowering does not know: the structurizer's regions are
/// `emit.rs`'s and the thunk is built from an op. Whole-function is the conservative answer in the
/// right direction — it snapshots in a few more places than v1 and never in fewer — and
/// snapshotting is what keeps a loop-reused environment captured by value.
fn function_has_cycle(cx: &JsCx<'_>) -> bool {
    let n = cx.func.blocks.len();
    // 0 = unvisited, 1 = on the stack, 2 = done. An edge to a block on the stack is a back edge.
    let mut state = vec![0u8; n];
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for start in 0..n {
        if state[start] != 0 {
            continue;
        }
        state[start] = 1;
        stack.push((start, 0));
        while let Some((block, next)) = stack.pop() {
            let successors: Vec<usize> = cx.func.blocks[block]
                .term
                .successors()
                .into_iter()
                .map(|id| id.0 as usize)
                .collect();
            if next >= successors.len() {
                state[block] = 2;
                continue;
            }
            stack.push((block, next + 1));
            let successor = successors[next];
            if successor >= n {
                continue;
            }
            match state[successor] {
                0 => {
                    state[successor] = 1;
                    stack.push((successor, 0));
                }
                1 => return true,
                _ => {}
            }
        }
    }
    false
}

/// The JavaScript name, the splatted arity and whether it takes an environment, of the body a
/// closure or function-item value calls.
fn closure_item(
    cx: &JsCx<'_>,
    ty: TypeId,
    convert_params_in: bool,
    convert_result_out: bool,
) -> Res<(String, usize, Vec<TypeId>, Option<TypeId>, bool)> {
    let peeled = cx.table.peeled(ty);
    let (definition, sig_id, has_environment) = match value::kind(cx.table, peeled) {
        Some(TypeKind::Closure { def, .. }) => (*def, None, true),
        Some(TypeKind::FnDef { def, sig, .. }) => (*def, Some(*sig), false),
        _ => {
            return Err(decline(
                OpClass::Host,
                "a host marker was given a value that is neither a closure nor a function item",
            ));
        }
    };
    let item = ItemId {
        definition,
        instantiation: peeled.0,
    };
    let Some(entry) = cx.items.get(&item) else {
        return Err(decline(
            OpClass::Host,
            "the closure a host marker was given has no item in this program",
        ));
    };
    // A closure body takes its environment and then the elements a caller splats out of the
    // `"rust-call"` tuple; `AbiSignature::params` already counts the tuple as its arity.
    let arity = entry
        .params
        .saturating_sub(usize::from(has_environment))
        .saturating_sub(usize::from(entry.caller_location));
    if !crate::lower::bstr::enabled() || (!convert_params_in && !convert_result_out) {
        return Ok((entry.name.clone(), arity, Vec::new(), None, has_environment));
    }
    let mut result_ty = entry.result;
    let param_tys = if !convert_params_in {
        if !has_environment {
            if let Some(sig) = sig_id.and_then(|id| cx.interns.sigs.get(id)) {
                if sig.spread_arity.is_none() && sig.inputs.len() >= arity {
                    result_ty = Some(sig.output);
                }
            }
        }
        Vec::new()
    } else if has_environment {
        let user_end = entry
            .arguments
            .len()
            .saturating_sub(usize::from(entry.caller_location));
        let user_args = entry.arguments.get(1..user_end).unwrap_or(&[]);
        if user_args.len() == 1 {
            match value::kind(cx.table, cx.table.peeled(user_args[0])) {
                Some(TypeKind::Tuple(elems)) if elems.len() == arity => elems.clone(),
                _ if arity == 1 => vec![user_args[0]],
                _ if arity == 0 => Vec::new(),
                _ => {
                    return Err(decline(
                        OpClass::Host,
                        "closure parameter shape disagrees with its arity",
                    ));
                }
            }
        } else if user_args.len() == arity {
            user_args.to_vec()
        } else if arity == 0 {
            Vec::new()
        } else {
            return Err(decline(
                OpClass::Host,
                "closure parameter count disagrees with its arity",
            ));
        }
    } else if let Some(sig) = sig_id.and_then(|id| cx.interns.sigs.get(id)) {
        if sig.spread_arity.is_none() && sig.inputs.len() >= arity {
            result_ty = Some(sig.output);
            sig.inputs[..arity].to_vec()
        } else if entry.arguments.len() >= arity {
            entry.arguments[..arity].to_vec()
        } else {
            return Err(decline(
                OpClass::Host,
                "function item parameter count disagrees with its arity",
            ));
        }
    } else if entry.arguments.len() >= arity {
        entry.arguments[..arity].to_vec()
    } else {
        return Err(decline(
            OpClass::Host,
            "function item parameter count disagrees with its arity",
        ));
    };
    Ok((
        entry.name.clone(),
        arity,
        param_tys,
        result_ty,
        has_environment,
    ))
}

/// The drop glue of a value a host marker takes ownership of, or `None` when the glue is empty
/// (v1 asked `Instance::resolve_drop_glue` and looked for `ShimKind::DropGlue(_, None)`).
fn glue_of(cx: &JsCx<'_>, ty: TypeId) -> Res<Option<String>> {
    let peeled = cx.table.peeled(ty);
    if !cx
        .table
        .get(peeled)
        .is_some_and(|descriptor| descriptor.layout.needs_drop)
    {
        return Ok(None);
    }
    match cx.drop_glue.get(&peeled) {
        Some(glue) => Ok(Some(glue.clone())),
        None => Err(decline(
            OpClass::Host,
            "a value a host marker takes ownership of needs dropping, and this program has no \
             drop glue item for its type",
        )),
    }
}

/// The one-shot drop callback of a closure a host marker takes ownership of, or `None` when its
/// drop glue is empty (v1 `host.rs::host_closure_drop`).
fn closure_drop(cx: &JsCx<'_>, closure: ValueId) -> Res<Option<Expr>> {
    let Some(glue) = glue_of(cx, cx.ty_of(closure)?)? else {
        return Ok(None);
    };
    Ok(Some(jsast::arrow(
        Vec::new(),
        jsast::call(jsast::id(glue), vec![cx.expr_of(closure)?]),
    )))
}

/// `spawn`/`spawnowner`/the Promise terminals: the monomorphic poller backed by native
/// JavaScript task state (v1 `host.rs::codegen_future_driver`/`codegen_future_promise`).
///
/// The poller and the drop callback are the concrete future's own: `poll_once::<F>` /
/// `poll_result::<F>` are the marker's sibling instances the encoder recorded on the payload
/// (`HostPayload::helper`), and the glue comes from the program's drop glue index.
fn future_driver(cx: &JsCx<'_>, op: &Op, marker: &str, args: &[ValueId]) -> Res<Expr> {
    let promise = matches!(marker, "futurepromise" | "futurenativepromise");
    let native = marker == "futurenativepromise";
    let with_owner = matches!(marker, "spawnowner" | "spawnowner.v2");
    let (future, owner) = match (with_owner, args) {
        (false, [future]) => (*future, None),
        (true, [future, owner]) => (*future, Some(*owner)),
        (false, _) => {
            return Err(decline(
                OpClass::Host,
                format!("`{HOST_PREFIX}{marker}` takes one future"),
            ))
        }
        (true, _) => {
            return Err(decline(
                OpClass::Host,
                format!("`{HOST_PREFIX}{marker}` takes a future and an owner"),
            ));
        }
    };
    let helper = match &op.kind {
        OpKind::Call {
            host: Some(payload),
            ..
        } => payload.helper(
            cx.interns,
            if promise { "poll_result" } else { "poll_once" },
        ),
        _ => None,
    };
    let Some(poll_name) = helper.and_then(|helper| cx.interns.str(helper.link_name)) else {
        return Err(decline(
            OpClass::Host,
            format!("`{HOST_PREFIX}{marker}` records no `poll_once`/`poll_result` helper instance"),
        ));
    };

    // The task state holds the future in its `future` field; a direct future lives in a slot of
    // that record, so the poller is handed the slot rather than the value.
    let future_ty = cx.ty_of(future)?;
    let state = jsast::id("$s");
    let future_ref = match cx.table.is_indirect(future_ty) {
        true => jsast::member(state, "future"),
        false => ptr::slot(state, jsast::string("future")),
    };
    let poll = jsast::arrow(
        vec!["$s".to_owned(), "$w".to_owned()],
        jsast::call(
            jsast::id(poll_name.to_owned()),
            vec![future_ref.clone(), jsast::id("$w")],
        ),
    );
    let drop = match glue_of(cx, future_ty)? {
        Some(glue) => jsast::arrow(
            vec!["$s".to_owned()],
            jsast::call(jsast::id(glue), vec![future_ref]),
        ),
        None => jsast::undefined(),
    };
    let future = cx.expr_of(future)?;
    if promise {
        return Ok(jsast::rt_call(
            if native {
                "host_future_async"
            } else if cx
                .physical
                .is_some_and(|(_, plan)| plan.ready_futures.contains(&future_ty))
            {
                "host_future_ready"
            } else {
                "host_future_promise"
            },
            vec![future, poll, drop],
        ));
    }
    // The owner is passed as a value, separately from the re-entry function: inside a deferred
    // arrow it would be captured by binding rather than by value.
    let (owner, runner) = match owner {
        Some(owner) => (cx.expr_of(owner)?, super::template::dom("runWithOwner")),
        None => (jsast::undefined(), jsast::undefined()),
    };
    let mut arguments = vec![
        future,
        poll,
        drop,
        jsast::boolean(with_owner),
        owner,
        runner,
    ];
    if marker == "spawnowner.v2" {
        arguments.push(super::template::dom("onCleanup"));
    }
    Ok(jsast::rt_call("host_spawn", arguments))
}

/// Applies the representation rules a typed JavaScript result must satisfy before Rust sees it
/// (v1 `js_extern.rs::host_boundary_value`).
///
/// Under `js-str=bytes` every `str` the result reaches is host text becoming a Rust `str`:
/// UTF-8 encoded into its byte string, wherever it sits in the value (`bstr::host_convert`), or
/// the lowering declines with the reason when it cannot be rebuilt exactly.
fn boundary_value(cx: &JsCx<'_>, ty: TypeId, value: Expr) -> Res<Expr> {
    if crate::lower::bstr::enabled() && crate::lower::bstr::reaches_str(cx.table, ty) {
        return crossing(cx, ty, value, crate::lower::bstr::Crossing::In);
    }
    Ok(match value::float_bits(cx.table, ty) {
        Some(32) => jsast::fround(value),
        _ => value,
    })
}

/// What the host is handed for the Rust value `arg`, spelled `value`. Under `js-str=bytes` every
/// `str` it reaches — or a `String` the plan keeps as its host string — is decoded to the host
/// text it encodes (`__rt.bstr_out`, `bstr::host_convert`); everything else crosses as it is.
fn host_argument(cx: &JsCx<'_>, arg: ValueId, value: Expr) -> Res<Expr> {
    if !crate::lower::bstr::enabled() || cx.is_known_ascii(arg) {
        return Ok(value);
    }
    let carrier = cx
        .func
        .values
        .get(arg.0 as usize)
        .map(|v| v.carrier)
        .unwrap_or(rcgjs_object::jir::repr::Carrier::Baseline);
    if carrier == rcgjs_object::jir::repr::Carrier::HostString {
        return Ok(crate::lower::bstr::host_out(value));
    }
    let ty = cx.ty_of(arg)?;
    crate::lower::bstr::host_convert_carrier(
        cx.table,
        cx.interns,
        ty,
        carrier,
        value,
        crate::lower::bstr::Crossing::Out,
    )
    .map_err(|why| {
        decline(
            OpClass::Host,
            format!("`js-str=bytes` cannot convert this value at the host boundary: {why}"),
        )
    })
}

/// [`crate::lower::bstr::host_convert`] at a host crossing, its refusal a decline that names it.
pub(super) fn crossing(
    cx: &JsCx<'_>,
    ty: TypeId,
    value: Expr,
    direction: crate::lower::bstr::Crossing,
) -> Res<Expr> {
    crate::lower::bstr::host_convert(cx.table, cx.interns, ty, value, direction).map_err(|why| {
        decline(
            OpClass::Host,
            format!("`js-str=bytes` cannot convert this value at the host boundary: {why}"),
        )
    })
}

// ---------------------------------------------------------------------------------------------
// `js!{}`
// ---------------------------------------------------------------------------------------------

/// Lowers a call to a `js!{}` marker (v1 `js_block.rs::codegen_js_block`).
fn js_block(cx: &JsCx<'_>, op: &Op, bytes: &[u8], args: &[ValueId]) -> Res<Expr> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| decline(OpClass::Host, "a `js!{}` block section is not UTF-8"))?;
    let block = Block::decode(text).map_err(|why| decline(OpClass::Host, why))?;
    if args.len() != block.slots {
        return Err(decline(
            OpClass::Host,
            format!(
                "a `js!{{}}` block takes {} capture(s) and was called with {}",
                block.slots,
                args.len()
            ),
        ));
    }
    let value = match block.slots {
        0 => jsast::raw_expr(block.text),
        slots => {
            let captures: Res<Vec<Expr>> = args
                .iter()
                .map(|arg| host_argument(cx, *arg, cx.expr_of(*arg)?))
                .collect();
            let params = (0..slots).map(block::slot_name).collect();
            jsast::call(jsast::arrow(params, jsast::raw_expr(block.text)), captures?)
        }
    };
    // The block's value is host JavaScript: a `str` result is host text (`js-str=bytes`).
    match op.result {
        Some(result) if crate::lower::bstr::enabled() => {
            boundary_value(cx, cx.ty_of(result)?, value)
        }
        _ => Ok(value),
    }
}

// ---------------------------------------------------------------------------------------------
// `#[js_extern]`
// ---------------------------------------------------------------------------------------------

/// The expression that reads `steps` off `root`, one member at a time
/// (v1 `js_extern.rs::walk`).
fn walk(root: Expr, steps: &[&str]) -> Expr {
    steps.iter().fold(root, |object, step| {
        jsast::member(object, (*step).to_owned())
    })
}

/// Lowers a call to a `#[js_extern]` declaration (v1 `js_extern.rs::codegen_js_extern`).
fn js_extern(cx: &JsCx<'_>, op: &Op, bytes: &[u8], args: &[ValueId]) -> Res<Expr> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| decline(OpClass::Host, "a `#[js_extern]` descriptor is not UTF-8"))?;
    let descriptor = Descriptor::decode(text).map_err(|why| decline(OpClass::Host, why))?;

    let required = descriptor.required_arguments();
    if args.len() < required {
        return Err(decline(
            OpClass::Host,
            format!(
                "a `#[js_extern]` declaration acts on {required} argument(s) and was called with {}",
                args.len()
            ),
        ));
    }
    let lowered: Vec<Expr> = args
        .iter()
        .map(|arg| host_argument(cx, *arg, cx.expr_of(*arg)?))
        .collect::<Res<Vec<Expr>>>()?;
    let (setup, arguments) = rcgjs_object::evaluation::OperandEvaluation::Materialize
        .capture(lowered, |value| cx.temp(value));
    js_extern_value(cx, op, &descriptor, required, &arguments).and_then(|value| {
        super::call::Lowered::Value(setup, value)
            .into_expr()
            .ok_or_else(|| decline(OpClass::Host, "host argument capture is not an expression"))
    })
}

fn js_extern_value(
    cx: &JsCx<'_>,
    op: &Op,
    descriptor: &Descriptor,
    required: usize,
    arguments: &[Expr],
) -> Res<Expr> {
    let (acted_on, passed) = arguments.split_at(required);

    let mut operands = acted_on.iter();
    let root = match descriptor.acts_on_argument() {
        true => operands.next().cloned().unwrap_or_else(jsast::undefined),
        false => js_binding(descriptor),
    };
    let segments = descriptor.segments();
    let steps = match descriptor.acts_on_argument() {
        true => &segments[..],
        false => segments.get(1..).unwrap_or_default(),
    };

    let value = match descriptor.shape {
        Shape::Call | Shape::Method => match steps.split_last() {
            Some((last, before)) => {
                jsast::method_call(walk(root, before), (*last).to_owned(), passed.to_vec())
            }
            None => jsast::call(root, passed.to_vec()),
        },
        Shape::New => jsast::new(walk(root, steps), passed.to_vec()),
        Shape::Get => walk(root, steps),
        Shape::Set => match steps.split_last() {
            Some((last, before)) => jsast::assign(
                jsast::member(walk(root, before), (*last).to_owned()),
                operands.next().cloned().unwrap_or_else(jsast::undefined),
            ),
            None => {
                return Err(decline(
                    OpClass::Host,
                    "a `#[js_extern]` `set` declaration writes no property",
                ));
            }
        },
        Shape::Index => jsast::index(
            walk(root, steps),
            operands.next().cloned().unwrap_or_else(jsast::undefined),
        ),
        Shape::IndexSet => {
            let key = operands.next().cloned().unwrap_or_else(jsast::undefined);
            let written = operands.next().cloned().unwrap_or_else(jsast::undefined);
            jsast::assign(jsast::index(walk(root, steps), key), written)
        }
        Shape::Has => jsast::method_call(
            jsast::id("Reflect"),
            "has",
            vec![
                walk(root, steps),
                operands.next().cloned().unwrap_or_else(jsast::undefined),
            ],
        ),
        Shape::ComputedMethod => jsast::call(
            jsast::index(
                walk(root, steps),
                operands.next().cloned().unwrap_or_else(jsast::undefined),
            ),
            passed.to_vec(),
        ),
    };

    let result_ty = match op.result {
        Some(result) => cx.ty_of(result)?,
        // A declaration whose result is unused still performs its operation.
        None => return Ok(value),
    };

    if descriptor.fallible {
        if descriptor.nullable {
            return Err(decline(
                OpClass::Host,
                "a `#[js_extern]` declaration cannot be both fallible and nullable",
            ));
        }
        // The destination a `#[js(fallible)]` declaration promises
        // (v1 `js_extern.rs::fallible_result`): a `Result<T, mires_js::JsValue>`, and nothing
        // else. A lookalike error type would silently receive a JavaScript exception it has no
        // representation for. The type table says which crate a nominal type belongs to through
        // `PinnedBoundary::Host`, which is set for every `mires_js` type.
        let Some(ok_ty) = result_value_ty(cx, result_ty) else {
            return Ok(cx.zombie(
                "`#[js(fallible)]` returns a `Result<T, mires_js::JsValue>`, and this                  declaration does not"
                    .to_owned(),
            ));
        };
        if !fallible_error_is_host_value(cx, result_ty) {
            return Ok(cx.zombie(
                "`#[js(fallible)]` returns a `Result<T, mires_js::JsValue>`, and this                  declaration returns another error type"
                    .to_owned(),
            ));
        }
        let value = boundary_value(cx, ok_ty, value)?;
        return Ok(jsast::rt_call(
            "host_try",
            vec![jsast::arrow(Vec::new(), value)],
        ));
    }
    if !descriptor.nullable {
        return boundary_value(cx, result_ty, value);
    }
    nullable(cx, result_ty, value)
}

/// What a `#[js_extern]` path is rooted at when it is not rooted at an argument: the module's
/// imported binding, or a global of the same name (v1 `js_extern.rs::js_binding`).
///
/// The `ItemKind::Import` item the binding needs is already in the program: the baseline lowering
/// of this very call interned it, and the JIR lane only has to name it the same way.
fn js_binding(descriptor: &Descriptor) -> Expr {
    let head = descriptor
        .segments()
        .first()
        .copied()
        .unwrap_or_default()
        .to_owned();
    if descriptor.root() != Root::Module {
        return jsast::id(head);
    }
    jsast::id(extern_import(&descriptor.module, &head))
}

/// `#[js(nullable)]`: `null` and `undefined` become `None`, anything else `Some`
/// (v1 `js_extern.rs::nullable`).
fn nullable(cx: &JsCx<'_>, ty: TypeId, value: Expr) -> Res<Expr> {
    let peeled = cx.table.peeled(ty);
    let variants = cx.table.variants(peeled);
    let index = |name: &str| -> Option<u32> {
        variants
            .iter()
            .position(|variant| value::variant_name(cx.interns, variant) == name)
            .map(|index| index as u32)
    };
    let (Some(some), Some(none)) = (index("Some"), index("None")) else {
        return Err(decline(
            OpClass::Host,
            "a `#[js(nullable)]` declaration returns an `Option`, and this one does not",
        ));
    };
    let value_ty = match value::kind(cx.table, peeled) {
        Some(TypeKind::Adt { args, .. }) => args.first().copied(),
        _ => None,
    };
    let value_ty = value_ty.ok_or_else(|| {
        decline(
            OpClass::Host,
            "a `#[js(nullable)]` declaration returns an `Option`, whose value type is not in the \
             type table",
        )
    })?;
    // Always a temporary, as v1's `nullable` is: both arms read the value, and a declared
    // interface may be called for its effects as well as its answer, so evaluating it twice
    // would perform it twice.
    let (_, held) = cx.temp(value.clone());
    let setup = Some(jsast::assign(held.clone(), value));
    let some = value::enum_value(
        cx.table,
        cx.interns,
        peeled,
        some,
        vec![("_0".to_owned(), boundary_value(cx, value_ty, held.clone())?)],
    )
    .ok_or_else(|| decline(OpClass::Host, "an `Option` with no enum representation"))?;
    let none = value::enum_value(cx.table, cx.interns, peeled, none, Vec::new())
        .ok_or_else(|| decline(OpClass::Host, "an `Option` with no enum representation"))?;
    let chosen = jsast::cond(jsast::binary(BinOp::Eq, held, jsast::null()), none, some);
    Ok(super::ptr::sequenced(setup, chosen))
}

/// The success type of a `Result<T, E>` (v1 `js_extern.rs::result_value_ty`).
fn result_value_ty(cx: &JsCx<'_>, ty: TypeId) -> Option<TypeId> {
    let peeled = cx.table.peeled(ty);
    let variants = cx.table.variants(peeled);
    if !variants.iter().any(|variant| variant.result_ok) {
        return None;
    }
    match value::kind(cx.table, peeled) {
        Some(TypeKind::Adt { args, .. }) => args.first().copied(),
        _ => None,
    }
}

/// Whether the `Err` half of a `Result` destination is a `mires_js` host value
/// (v1 `js_extern.rs::fallible_result`).
///
/// The type table records the crate a nominal type belongs to as
/// [`PinnedBoundary::Host`](rcgjs_object::jir::types::PinnedBoundary), which is what stands in
/// for v1's `def_path_str(..) == "mires_js::JsValue"`: a lookalike in the user's crate carries
/// no boundary at all, which is the case this guard exists for.
fn fallible_error_is_host_value(cx: &JsCx<'_>, result_ty: TypeId) -> bool {
    let peeled = cx.table.peeled(result_ty);
    let Some(TypeKind::Adt { args, .. }) = value::kind(cx.table, peeled) else {
        return false;
    };
    let Some(error) = args.get(1) else {
        return false;
    };
    // Asked of the type itself, never of its peeled representation: `JsValue` is a transparent
    // handle, and the handle's own type belongs to no crate in particular.
    cx.table.get(*error).is_some_and(|d| {
        matches!(
            d.meta.boundary,
            rcgjs_object::jir::types::PinnedBoundary::Host
        )
    })
}

/// The local a `#[js_extern]` module import binds, spelled exactly as v1's
/// `naming::extern_import` spells it.
pub fn extern_import(module: &str, name: &str) -> String {
    readable(&format!("ext::{name}"), &format!("{module}\u{0}{name}"))
}

// ---------------------------------------------------------------------------------------------
// `rcgjs.map.*`
// ---------------------------------------------------------------------------------------------

/// The `__rt` member a collection marker becomes (v1 `map.rs::rt_member`).
fn map_member(marker: &str) -> Option<&'static str> {
    Some(match marker {
        "has" => "map_has",
        "set" => "map_set",
        "get" | "getmut" => "map_get",
        "del" => "map_del",
        "keys" => "map_keys",
        _ => return None,
    })
}

/// Which argument of a marker is the key (v1 `map.rs::key_argument`).
fn map_key_argument(marker: &str) -> Option<usize> {
    match marker {
        "has" | "set" | "get" | "getmut" | "del" => Some(1),
        _ => None,
    }
}

/// Lowers a call to one of the `mires-js` collection markers (v1 `map.rs::codegen_map_marker`).
fn map_marker(cx: &JsCx<'_>, _op: &Op, marker: &str, args: &[ValueId]) -> Res<Expr> {
    let Some(member) = map_member(marker) else {
        return Err(decline(
            OpClass::Host,
            format!(
                "`{MAP_PREFIX}{marker}` is not a `mires-js` collection marker this backend knows"
            ),
        ));
    };
    let key = map_key_argument(marker);
    let mut lowered = Vec::with_capacity(args.len());
    for (index, arg) in args.iter().enumerate() {
        let value = cx.expr_of(*arg)?;
        if Some(index) != key {
            lowered.push(value);
            continue;
        }
        let ty = cx.ty_of(*arg)?;
        map_key_class(cx, ty)?;
        lowered.push(match value::is_slot(cx.table, ty) {
            true => ptr::slot_element(value),
            false => value,
        });
    }
    Ok(jsast::rt_call(member, lowered))
}

/// Checks that a marker's key argument is a reference to a class a host `Map` compares by value
/// (v1 `map.rs::key_class_error`).
fn map_key_class(cx: &JsCx<'_>, key_ty: TypeId) -> Res<()> {
    let Some(TypeKind::Reference { pointee, .. }) = value::kind(cx.table, cx.table.peeled(key_ty))
    else {
        return Err(decline(
            OpClass::Host,
            "a `rcgjs.map.` marker takes its key by reference, and this argument is not one",
        ));
    };
    let class = cx.table.peeled(*pointee);
    match value::kind(cx.table, class) {
        Some(TypeKind::Bool | TypeKind::Char | TypeKind::Integer { .. } | TypeKind::Str) => Ok(()),
        Some(TypeKind::Float { .. }) => Err(decline(
            OpClass::Host,
            "a float cannot be a map key: `core` gives the floats no `Eq` for the reason that \
             `NaN != NaN`, and a host `Map` answers a third way again",
        )),
        _ => Err(decline(
            OpClass::Host,
            "this type cannot be a map key: a host `Map` compares keys with SameValueZero, which \
             is identity for anything that is not an integer, a `bool`, a `char` or a `str`, so \
             an equal key built later would never find this entry",
        )),
    }
}

// ---------------------------------------------------------------------------------------------
// Naming
// ---------------------------------------------------------------------------------------------

/// v1 `naming.rs`'s `PATH_SEP`.
const PATH_SEP: char = '$';
/// v1 `naming.rs`'s `HASH_SEP`.
const HASH_SEP: &str = "$h";

/// v1 `naming.rs::push_sep`.
fn push_sep(out: &mut String, sep: char) {
    if out.is_empty() || out.ends_with('_') || out.ends_with(PATH_SEP) {
        return;
    }
    out.push(sep);
}

/// v1 `naming.rs::readable`: the name a synthetic item is interned under.
pub fn readable(path: &str, key: &str) -> String {
    let mut out = String::with_capacity(path.len() + HASH_SEP.len() + 16);
    let mut chars = path.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == ':' && chars.peek() == Some(&':') {
            chars.next();
            push_sep(&mut out, PATH_SEP);
        } else if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            push_sep(&mut out, '_');
        }
    }
    while out.ends_with('_') || out.ends_with(PATH_SEP) {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("item");
    }
    let head: Vec<char> = out.chars().take(2).collect();
    let needs_prefix = head[0] == PATH_SEP
        || head[0].is_ascii_digit()
        || (head[0] == '_' && head.get(1).is_some_and(|c| c.is_ascii_digit()));
    if needs_prefix {
        out.insert(0, 'x');
    }
    out.push_str(HASH_SEP);
    let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{:016x}", fnv1a(key)));
    out
}

/// FNV-1a, 64 bit (v1 `naming.rs::hash64`).
fn fnv1a(key: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// One item of the program, as a lowering that has to spell a call to it needs it: its JavaScript
/// name and its parameter shape.
#[derive(Clone, Debug)]
pub struct ItemEntry {
    pub name: String,
    /// `AbiSignature::parameters`: MIR arguments, a spread tuple counted as its arity, the caller
    /// `Location` counted when `caller_location` is set.
    pub params: usize,
    /// Whether the item supplied an ABI, including a known zero-parameter ABI.
    pub abi_known: bool,
    pub result: Option<TypeId>,
    /// The Rust argument types of the body, when it has one: what a `js-str=bytes` closure thunk
    /// checks the host's arguments against.
    pub arguments: Vec<TypeId>,
    pub caller_location: bool,
    /// The `DefinitionId` of `core::panic::Location` when this item directly calls the
    /// `caller_location` intrinsic, letting non-intrinsic `caller_location_value` sites
    /// disambiguate `core::panic::Location` from user-defined `Location` structs.
    pub caller_location_def: Option<DefinitionId>,
    /// Whether this item belongs to an authenticated `core` crate (`crate_name == "core"`).
    pub is_core: bool,
    /// Direct SSA parameters mentioned by the body before optimization. This conservative set
    /// lets special call boundaries reject a live argument lane they cannot represent yet.
    pub used_params: BTreeSet<u16>,
    /// The body's `BodySemantic::Standard` tag, when it has one: the exact linked identity the
    /// call-site format kernel resolves a callee by (`lower/semantic_ops/fmt_kernel.rs`).
    pub standard: Option<crate::jir::semantic::StandardOperation>,
    /// The validated effects class a keyed-list key extractor may rely on.
    pub keyed_purity: Option<KeyedPurity>,
    /// The exact caller-visible parameters read by a `Read` keyed callable. `None` means some
    /// read came from a global, spill, capture of unknown provenance, or unresolved callee.
    pub keyed_read_params: Option<BTreeSet<u16>>,
    /// Whether the keyed callable's validated effects summary can unwind.
    pub keyed_may_unwind: Option<bool>,
    /// Whether every reachable row operation is creation-only in the initial static lane.
    pub keyed_row_capture: Option<KeyedRowCapture>,
    /// Parameters for which the linker proved that the stable callable proxy used by the first
    /// live-slot lane cannot escape or reach an operation with object-identity semantics.
    pub keyed_proxy_safe_params: Option<BTreeSet<u16>>,
}

/// Whether a direct callable reads or changes anything beyond its arguments.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyedPurity {
    Const,
    Read,
    Effect,
}

/// The row-capture class proven from the direct whole-program call graph.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyedRowCapture {
    Static,
    Live,
}

/// Every item of the program by its semantic identity: what a closure body is called and how many
/// arguments it takes, which v1 asked `Instance::new_raw` + `namer.fn_name` for.
pub type ItemIndex = BTreeMap<ItemId, ItemEntry>;

/// The empty index a [`JsCx`] built without a driver carries.
pub static EMPTY_ITEMS: &ItemIndex = &BTreeMap::new();

/// The drop glue of every type the program drops anywhere, by that type.
///
/// v1 asked `Instance::resolve_drop_glue`; a JIR-S body records its own identity
/// (`BodySemantic::DropGlue(ty)`), so the glue of a closure or a future a host marker takes
/// ownership of is nameable here even though no `Drop` op in *this* body names it.
pub type DropGlueIndex = BTreeMap<TypeId, String>;

/// The empty drop-glue index a [`JsCx`] built without a driver carries.
pub static EMPTY_DROP_GLUE: &DropGlueIndex = &BTreeMap::new();
