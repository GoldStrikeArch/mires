//! The small dynamic host ABI exposed by `mires-js`.

use rustc_hir::def_id::DefId;
use rustc_middle::mir;
use rustc_middle::ty::{self, Instance, Ty, TyCtxt};
use rustc_span::Spanned;

use crate::base::FnCx;
use crate::jsast::{self, BinOp, Expr, Stmt, UnOp};

/// The `link_section` prefix every host marker carries.
pub(crate) const HOST_PREFIX: &str = "rcgjs.host.";

/// The marker `def_id` carries, if it belongs to this ABI.
pub(crate) fn host_section(tcx: TyCtxt<'_>, def_id: DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section
        .as_str()
        .strip_prefix(HOST_PREFIX)
        .map(str::to_owned)
}

/// Whether a host marker has a safe Rust body that optimizer modes may replace.
pub(crate) fn is_async_marker(marker: &str) -> bool {
    matches!(
        marker,
        "spawn" | "spawnowner" | "spawnowner.v2" | "futurepromise" | "futurenativepromise"
    )
}

impl<'tcx> FnCx<'_, 'tcx> {
    /// Lowers one `mires-js` host marker.
    pub(crate) fn codegen_host_marker(
        &self,
        marker: &str,
        def_id: DefId,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        if matches!(marker, "spawn" | "spawnowner" | "spawnowner.v2") {
            return self.codegen_future_driver(
                def_id,
                args,
                marker != "spawn",
                marker == "spawnowner.v2",
            );
        }
        if marker == "futurepromise" || marker == "futurenativepromise" {
            return self.codegen_future_promise(
                def_id,
                args,
                destination,
                marker == "futurenativepromise",
            );
        }
        let values: Vec<Expr> = args
            .iter()
            .map(|arg| self.at(arg.span, || self.codegen_operand(&arg.node)))
            .collect();
        if let Some(operation) = rcgjs_object::host::HostOperation::from_marker(marker) {
            return match operation.lower(values) {
                Ok(expression) => self.write_place(destination, expression),
                Err(error) => self.host_zombie(marker, &error),
            };
        }
        let one = || {
            values
                .first()
                .cloned()
                .ok_or_else(|| format!("`{HOST_PREFIX}{marker}` takes one argument"))
        };
        let expression = match marker {
            "iserror" => {
                one().map(|value| jsast::call(jsast::id("__rt.host_is_error"), vec![value]))
            }
            "base64url" => one().map(|value| jsast::rt_call("host_base64url", vec![value])),
            "base64urlnative" => one().map(|value| {
                jsast::method_call(
                    value,
                    "toBase64",
                    vec![jsast::object(vec![
                        ("alphabet".to_owned(), jsast::string("base64url")),
                        ("omitPadding".to_owned(), jsast::boolean(true)),
                    ])],
                )
            }),
            "base64urlsupported" => {
                if !values.is_empty() {
                    Err(format!("`{HOST_PREFIX}{marker}` takes no arguments"))
                } else {
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
            }
            "badread" => {
                if !values.is_empty() {
                    return self.host_zombie(marker, "takes no arguments");
                }
                return vec![jsast::throw(jsast::new(
                    jsast::id("TypeError"),
                    vec![jsast::string(
                        "host value does not match its Rust representation",
                    )],
                ))];
            }
            "read" => one().map(|value| {
                let ty = self.monomorphize(destination.ty(&self.mir.local_decls, self.tcx).ty);
                match ty.kind() {
                    // A JavaScript Number has f64 precision. Reading one as Rust `f32` must round
                    // at the boundary, before a comparison or move can observe extra precision.
                    ty::Float(ty::FloatTy::F32) => jsast::fround(value),
                    _ => value,
                }
            }),
            "callback0" | "callback1" | "callback2" | "callback3" | "callback4" => {
                let [closure] = args else {
                    return self.host_zombie(marker, "takes one closure");
                };
                match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                    Ok((arrow, _)) => Ok(arrow),
                    Err(error) => Err(error),
                }
            }
            "try" => {
                let [closure] = args else {
                    return self.host_zombie(marker, "takes one closure");
                };
                match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                    Ok((arrow, _)) => {
                        let mut arguments = vec![arrow];
                        arguments.extend(self.host_closure_drop(&closure.node));
                        Ok(jsast::rt_call("host_try", arguments))
                    }
                    Err(error) => Err(error),
                }
            }
            "throw" => {
                let Ok(value) = one() else {
                    return self.host_zombie(marker, "takes one value");
                };
                return vec![jsast::throw(value)];
            }
            "localwaker" => {
                let [closure] = args else {
                    return self.host_zombie(marker, "takes one closure");
                };
                match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                    Ok((arrow, _)) => Ok(jsast::rt_call("host_local_waker", vec![arrow])),
                    Err(error) => Err(error),
                }
            }
            "microtask" | "task" => {
                let [closure] = args else {
                    return self.host_zombie(marker, "takes one closure");
                };
                return match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                    Ok((arrow, _)) => vec![jsast::expr_stmt(jsast::rt_call(
                        if marker == "task" {
                            "host_task"
                        } else {
                            "microtask"
                        },
                        vec![arrow],
                    ))],
                    Err(error) => vec![jsast::expr_stmt(self.zombie(error))],
                };
            }
            "settled" => {
                let [value, closure] = args else {
                    return self.host_zombie(marker, "takes a value and a closure");
                };
                let value = self.at(value.span, || self.codegen_operand(&value.node));
                return match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                    Ok((arrow, _)) => {
                        vec![jsast::expr_stmt(jsast::rt_call(
                            "settled",
                            vec![value, arrow],
                        ))]
                    }
                    Err(error) => vec![jsast::expr_stmt(self.zombie(error))],
                };
            }
            "promise" => {
                let [closure] = args else {
                    return self.host_zombie(marker, "takes one closure");
                };
                match self.at(closure.span, || self.closure_thunk(&closure.node)) {
                    Ok((arrow, _)) => Ok(jsast::new(jsast::id("Promise"), vec![arrow])),
                    Err(error) => Err(error),
                }
            }
            _ => Err(format!(
                "`{HOST_PREFIX}{marker}` is not a host marker this backend knows"
            )),
        };
        match expression {
            Ok(expression) => self.write_place(destination, expression),
            Err(error) => vec![jsast::expr_stmt(self.zombie(error))],
        }
    }

    /// Emits one monomorphic future driver backed by native JavaScript task state.
    fn codegen_future_driver(
        &self,
        marker: DefId,
        args: &[Spanned<mir::Operand<'tcx>>],
        with_owner: bool,
        cleanup_on_owner: bool,
    ) -> Vec<Stmt> {
        let (future, owner) = match (with_owner, args) {
            (false, [future]) => (future, None),
            (true, [future, owner]) => (future, Some(owner)),
            (false, _) => return self.host_zombie("spawn", "takes one future"),
            (true, _) => return self.host_zombie("spawnowner", "takes a future and an owner"),
        };
        let future_ty = self.operand_ty(&future.node);
        self.record_async_lane(future_ty, with_owner);
        let Some(poll) = self.host_future_helper(marker, "poll_once", future_ty) else {
            return self.host_zombie("spawn", "cannot resolve its `poll_once` helper");
        };

        let state = jsast::id("$s");
        let stored = jsast::member(state.clone(), "future");
        let future_ref = if crate::value::is_indirect(self.tcx, future_ty) {
            stored
        } else {
            crate::ptr::slot(state.clone(), jsast::string("future"))
        };
        let poll = jsast::arrow(
            vec!["$s".to_owned(), "$w".to_owned()],
            jsast::call(jsast::id(poll), vec![future_ref.clone(), jsast::id("$w")]),
        );

        let drop = self.host_future_drop(future_ty, future_ref);

        // Pass the owner as a value, separately from the re-entry function. Putting its operand
        // inside a deferred arrow would capture a loop-reused MIR local by binding, not by value.
        let (owner, runner) = match owner {
            Some(owner) => (
                self.at(owner.span, || self.codegen_operand(&owner.node)),
                crate::template::dom_import(self.cgu, "runWithOwner"),
            ),
            None => (jsast::undefined(), jsast::undefined()),
        };
        let future = self.at(future.span, || self.codegen_operand(&future.node));
        let mut arguments = vec![
            future,
            poll,
            drop,
            jsast::boolean(with_owner),
            owner,
            runner,
        ];
        if cleanup_on_owner {
            arguments.push(crate::template::dom_import(self.cgu, "onCleanup"));
        }
        vec![jsast::expr_stmt(jsast::rt_call("host_spawn", arguments))]
    }

    /// Emits a Promise terminal that polls one concrete future and settles from its `Result`.
    fn codegen_future_promise(
        &self,
        marker: DefId,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
        native: bool,
    ) -> Vec<Stmt> {
        let [future] = args else {
            return self.host_zombie("futurepromise", "takes one future");
        };
        let future_ty = self.operand_ty(&future.node);
        self.record_async_lane(future_ty, false);
        let Some(poll) = self.host_future_helper(marker, "poll_result", future_ty) else {
            return self.host_zombie("futurepromise", "cannot resolve its `poll_result` helper");
        };
        let state = jsast::id("$s");
        let stored = jsast::member(state.clone(), "future");
        let future_ref = if crate::value::is_indirect(self.tcx, future_ty) {
            stored
        } else {
            crate::ptr::slot(state.clone(), jsast::string("future"))
        };
        let poll = jsast::arrow(
            vec!["$s".to_owned(), "$w".to_owned()],
            jsast::call(jsast::id(poll), vec![future_ref.clone(), jsast::id("$w")]),
        );
        let drop = self.host_future_drop(future_ty, future_ref);
        let future = self.at(future.span, || self.codegen_operand(&future.node));
        self.write_place(
            destination,
            jsast::rt_call(
                if native {
                    "host_future_async"
                } else {
                    "host_future_promise"
                },
                vec![future, poll, drop],
            ),
        )
    }

    /// The concrete future's drop callback, or `undefined` for an empty glue shim.
    fn host_future_drop(&self, future: Ty<'tcx>, future_ref: Expr) -> Expr {
        let glue = Instance::resolve_drop_glue(self.tcx, future);
        match glue.def {
            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => jsast::undefined(),
            _ => jsast::arrow(
                vec!["$s".to_owned()],
                jsast::call(
                    jsast::id(self.cgu.namer.fn_name(glue).into_string()),
                    vec![future_ref],
                ),
            ),
        }
    }

    /// The closure's one-shot drop callback, omitted when its drop glue is empty.
    fn host_closure_drop(&self, closure: &mir::Operand<'tcx>) -> Option<Expr> {
        let closure_ty = self.operand_ty(closure);
        let glue = Instance::resolve_drop_glue(self.tcx, closure_ty);
        match glue.def {
            ty::InstanceKind::Shim(ty::ShimKind::DropGlue(_, None)) => None,
            _ => Some(jsast::arrow(
                Vec::new(),
                jsast::call(
                    jsast::id(self.cgu.namer.fn_name(glue).into_string()),
                    vec![self.codegen_operand(closure)],
                ),
            )),
        }
    }

    /// Resolves a generic helper declared beside a host marker for the marker's future type.
    fn host_future_helper(&self, marker: DefId, name: &str, future: Ty<'tcx>) -> Option<String> {
        let parent = self.tcx.parent(marker);
        let helper = self
            .tcx
            .module_children(parent)
            .iter()
            .find(|child| child.ident.name.as_str() == name)
            .and_then(|child| child.res.opt_def_id())?;
        let args = self.tcx.mk_args(&[future.into()]);
        let instance = Instance::try_resolve(self.tcx, crate::value::typing_env(), helper, args)
            .ok()
            .flatten()?;
        Some(self.cgu.namer.fn_name(instance).into_string())
    }

    /// Records the conservative lane selected for one async terminal.
    pub(crate) fn record_async_lane(&self, future: Ty<'tcx>, owner_aware: bool) {
        self.record_optimization_selection(
            "future-poller",
            "monomorphic-poller",
            "concrete Future poll and drop callbacks",
        );
        let eligibility = crate::async_analysis::promise_eligibility(self.tcx, future, owner_aware);
        let awaited = if eligibility.compiler_owned.is_empty() {
            String::new()
        } else {
            let owned: Vec<&str> = eligibility.compiler_owned.iter().copied().collect();
            format!("; awaits {}", owned.join(", "))
        };
        if eligibility.eligible {
            self.record_optimization_selection(
                "native-promise-continuations",
                "eligible-report-only",
                format!(
                    "{} concrete coroutine(s), {} canonical await loop(s), {} suspension \
                     point(s){awaited}",
                    eligibility.coroutines, eligibility.await_loops, eligibility.suspension_points
                ),
            );
        } else {
            self.record_optimization_miss_at(
                "native-promise-continuations",
                format!(
                    "{}; {} concrete coroutine(s), {} canonical await loop(s), {} suspension \
                     point(s){awaited}",
                    eligibility
                        .fallback
                        .expect("ineligible analysis has a reason"),
                    eligibility.coroutines,
                    eligibility.await_loops,
                    eligibility.suspension_points
                ),
                self.span(),
            );
        }
    }

    /// Emits a named marker error.
    fn host_zombie(&self, marker: &str, why: &str) -> Vec<Stmt> {
        vec![jsast::expr_stmt(
            self.zombie(format!("`{HOST_PREFIX}{marker}` {why}")),
        )]
    }
}
