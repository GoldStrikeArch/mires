//! `#[js_extern]`: calling a declared JavaScript interface.
//!
//! A program declares an interface with the `js-extern-macro` attribute, which writes one marker
//! function per declaration carrying the interface's [`Descriptor`] in its `link_section`. The
//! call is intercepted in [`crate::abi`]'s `codegen_call`, ahead of `is_foreign_item` and so ahead
//! of the `__rt` print shim a foreign call would otherwise reach, and this module emits the
//! JavaScript the descriptor names.
//!
//! # What crosses the boundary
//!
//! Arguments pass through: the value model already spells a number as a number, a `&str` as a
//! string and a `#[repr(C)]` struct as an object keyed by field name, so a declared interface
//! takes the arguments it was declared with and nothing is marshalled. What a shape adds is
//! position: a method, a property and an index act on their FIRST argument, which is therefore not
//! passed on.
//!
//! # Imports
//!
//! A declaration that names a module binds it with an [`ItemKind::Import`], the same mechanism the
//! DOM runtime's own imports use. The binding is named by [`crate::naming::extern_import`], which
//! keys on the module and the name together, so two modules exporting `Chart` bind two locals and
//! two crates importing the same `Chart` bind one.
//!
//! A declaration rooted at the global scope is emitted as the identifier itself and imports
//! nothing, which is what `new EventSource(url)` needs: the host already has the name.

use rustc_middle::mir;
use rustc_middle::ty::{self, TyCtxt};
use rustc_span::Spanned;

use crate::base::FnCx;
use crate::descriptor::{Descriptor, Root, Shape, PREFIX};
use crate::item::{ItemKind, JsItem, JsName, Linkage};
use crate::jsast::{self, Expr, Stmt};

/// The expression that reads `steps` off `root`, one member at a time.
///
/// A walk is emitted step by step rather than collapsed, because each step is a property read the
/// interface may be doing deliberately: `chart.data.datasets` reads `data` and then `datasets`,
/// and a library whose `data` getter has an effect would notice the difference.
fn walk(root: Expr, steps: &[&str]) -> Expr {
    steps.iter().fold(root, |object, step| {
        jsast::member(object, (*step).to_owned())
    })
}

/// The descriptor `def_id` carries, or `None` for anything that is not a declaration.
pub(crate) fn extern_section(tcx: TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section.as_str().strip_prefix(PREFIX).map(str::to_owned)
}

/// Declared globals read by this body must remain outside its local namespace.
pub(crate) fn global_names<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: ty::Instance<'tcx>,
    body: &mir::Body<'tcx>,
) -> std::collections::HashSet<String> {
    use rustc_middle::mir::visit::Visitor;

    struct Globals<'a, 'tcx> {
        tcx: TyCtxt<'tcx>,
        body: &'a mir::Body<'tcx>,
        names: std::collections::HashSet<String>,
    }
    impl Globals<'_, '_> {
        fn add(&mut self, definition: rustc_hir::def_id::DefId) {
            let Some(section) = extern_section(self.tcx, definition) else {
                return;
            };
            let Ok(descriptor) = Descriptor::decode(&section) else {
                return;
            };
            if descriptor.root() != Root::Module && !descriptor.acts_on_argument() {
                if let Some(head) = descriptor.segments().first() {
                    self.names.insert((*head).to_owned());
                }
            }
        }
    }
    impl<'tcx> Visitor<'tcx> for Globals<'_, 'tcx> {
        fn visit_operand(&mut self, operand: &mir::Operand<'tcx>, _: mir::Location) {
            if let ty::FnDef(definition, _) = operand.ty(&self.body.local_decls, self.tcx).kind() {
                self.add(*definition);
            }
        }
    }
    let mut globals = Globals {
        tcx,
        body,
        names: Default::default(),
    };
    globals.add(instance.def_id());
    globals.visit_body(body);
    globals.names
}

impl<'tcx> FnCx<'_, 'tcx> {
    /// Emits the host operation as this marker function's body.
    ///
    /// Call sites still inline the same operation. Native regions that pass sink arguments call
    /// this compiled body instead of the `unreachable!` placeholder the macro writes.
    pub(crate) fn codegen_js_extern_fn(&self, body: &str) -> Vec<Stmt> {
        let args: Vec<Spanned<mir::Operand<'tcx>>> = self
            .mir
            .args_iter()
            .map(|local| Spanned {
                node: mir::Operand::Copy(local.into()),
                span: self.mir.span,
            })
            .collect();
        let destination: mir::Place<'tcx> = mir::RETURN_PLACE.into();
        let mut stmts = self.codegen_js_extern(body, &args, destination);
        let (target, _) = self.codegen_place(destination);
        stmts.push(jsast::ret(self.read(&target)));
        stmts
    }

    /// Lowers a call to a `#[js_extern]` declaration.
    pub(crate) fn codegen_js_extern(
        &self,
        body: &str,
        args: &[Spanned<mir::Operand<'tcx>>],
        destination: mir::Place<'tcx>,
    ) -> Vec<Stmt> {
        let descriptor = match Descriptor::decode(body) {
            Ok(descriptor) => descriptor,
            Err(why) => return vec![jsast::expr_stmt(self.zombie(why))],
        };

        // The arity is checked on the declaration too, where the error is spanned; this is what
        // makes the lowering total for a declaration written by anything else.
        let required = descriptor.required_arguments();
        if args.len() < required {
            return vec![jsast::expr_stmt(self.zombie(format!(
                "`{PREFIX}{body}` acts on {required} argument(s) and was called with {}",
                args.len()
            )))];
        }

        // Rust arguments are evaluated before the host binding looks up its callee or
        // property. Compiler temporaries cannot be fused back across that lookup.
        let mut setup = Vec::new();
        let lowered: Vec<Expr> = args
            .iter()
            .map(|arg| {
                let value = self.at(arg.span, || self.codegen_operand(&arg.node));
                let (hold, value) = self.temp(value);
                setup.push(hold);
                value
            })
            .collect();
        let (acted_on, passed) = lowered.split_at(required);

        // The receiver, where the shape has one, is the first argument; the key and the value a
        // shape needs follow it, and everything after them passes through.
        let mut operands = acted_on.iter();
        let root = match descriptor.acts_on_argument() {
            true => operands.next().cloned().unwrap_or_else(jsast::undefined),
            false => self.js_binding(&descriptor),
        };
        // A shape rooted at a binding spent the path's first segment naming it. The macro rejects
        // such a shape with no name at all; `get` rather than an index keeps a descriptor written
        // by anything else from panicking here.
        let segments = descriptor.segments();
        let steps = match descriptor.acts_on_argument() {
            true => &segments[..],
            false => segments.get(1..).unwrap_or_default(),
        };

        let value = match descriptor.shape {
            // A call and a method are one operation written two ways: what differs is where the
            // path is rooted, which `root` has already answered. The last step is called as a
            // MEMBER rather than as a plain call, so `Chart.register(x)` keeps its receiver.
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
                    return vec![jsast::expr_stmt(
                        self.zombie(format!("`{PREFIX}{body}` writes no property")),
                    )];
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

        let value = if descriptor.fallible {
            if descriptor.nullable {
                return vec![jsast::expr_stmt(self.zombie(format!(
                    "`{PREFIX}{body}` cannot be both fallible and nullable"
                )))];
            }
            if let Err(why) = self.fallible_result(destination) {
                return vec![jsast::expr_stmt(self.zombie(why))];
            }
            let ok_ty = self.result_value_ty(destination).unwrap_or_else(|| {
                self.monomorphize(destination.ty(&self.mir.local_decls, self.tcx).ty)
            });
            let value = self.host_boundary_value(ok_ty, value);
            jsast::rt_call("host_try", vec![jsast::arrow(Vec::new(), value)])
        } else if !descriptor.nullable {
            let ty = self.monomorphize(destination.ty(&self.mir.local_decls, self.tcx).ty);
            self.host_boundary_value(ty, value)
        } else {
            match self.nullable(destination, value) {
                Ok((hold, wrapped)) => {
                    setup.push(hold);
                    wrapped
                }
                Err(why) => return vec![jsast::expr_stmt(self.zombie(why))],
            }
        };
        // Host getters and global bindings can throw or mutate state even when their
        // results are unused. Keep their evaluation out of the ordinary Rust-read queue.
        let (hold, value) = self.temp(value);
        setup.push(hold);
        setup.extend(self.write_place(destination, value));
        setup
    }

    /// Checks the destination promised by a `#[js(fallible)]` declaration.
    fn fallible_result(&self, destination: mir::Place<'tcx>) -> Result<(), String> {
        let tcx = self.tcx;
        let ty = self.monomorphize(destination.ty(&self.mir.local_decls, tcx).ty);
        let ty::Adt(def, args) = ty.kind() else {
            return Err(format!(
                "`#[js(fallible)]` returns a `Result<T, JsValue>`, and this returns `{ty}`"
            ));
        };
        if !def.is_enum() || !tcx.is_diagnostic_item(rustc_span::sym::Result, def.did()) {
            return Err(format!(
                "`#[js(fallible)]` returns a `Result<T, JsValue>`, and this returns `{ty}`"
            ));
        }
        let error = args.type_at(1);
        let ty::Adt(error_def, _) = error.kind() else {
            return Err(format!(
                "`#[js(fallible)]` returns a `Result<T, mires_js::JsValue>`, and this returns \
                 `{ty}`"
            ));
        };
        // A structural path identifies the same type inside its defining crate and through
        // downstream re-exports. Diagnostic paths depend on where they are printed.
        let is_host_value = tcx.crate_name(error_def.did().krate).as_str() == "mires_js"
            && tcx.def_path(error_def.did()).to_string_no_crate_verbose() == "::host::JsValue";
        if !is_host_value {
            return Err(format!(
                "`#[js(fallible)]` returns a `Result<T, mires_js::JsValue>`, and this returns \
                 `{ty}`"
            ));
        }
        Ok(())
    }

    /// What the path is rooted at when it is not rooted at an argument: the module's imported
    /// binding, or a global of the same name.
    ///
    /// Only the path's FIRST segment names it either way. `Chart.register` imports `Chart` and
    /// reaches `register` through it, because `import { Chart.register }` is not a thing anyone
    /// can write.
    fn js_binding(&self, descriptor: &Descriptor) -> Expr {
        let head = descriptor
            .segments()
            .first()
            .copied()
            .unwrap_or_default()
            .to_owned();
        // The descriptor's own answer rather than a second reading of its module, so a global and
        // an import cannot be told apart two ways that disagree.
        if descriptor.root() != Root::Module {
            return jsast::id(head);
        }

        let local = crate::naming::extern_import(&descriptor.module, &head);
        let js_name = JsName::new(local.clone());
        let source = descriptor.module.clone();
        self.cgu.intern(js_name.clone(), || {
            JsItem::new(
                js_name.clone(),
                ItemKind::Import,
                jsast::import_named(vec![(head.clone(), local.clone())], source.clone()),
                Vec::new(),
                Linkage {
                    fixed_name: Some(local.clone()),
                    root: false,
                    exported: false,
                },
                source.clone(),
            )
        });
        jsast::id(local)
    }

    /// `#[js(nullable)]`: `null` and `undefined` become `None`, anything else `Some`.
    ///
    /// The value is held in a temporary first, because both arms of the test read it and a
    /// declared interface may be called for its effects as well as its answer: evaluating the call
    /// twice would perform it twice.
    ///
    /// The two `Option` values are built by [`crate::value::enum_value`] from the destination's own
    /// type, so the encoding is the one the rest of the program reads and this lowering has no
    /// opinion about it. `== null` rather than `=== null` is what makes `undefined` a `None` too,
    /// which is the case that matters: a missing property reads as `undefined`, not as `null`.
    ///
    /// # Errors
    ///
    /// Returns why the destination is not an `Option`, so a declaration whose return type does not
    /// match its flag is a named zombie rather than a wrong value.
    fn nullable(&self, destination: mir::Place<'tcx>, value: Expr) -> Result<(Stmt, Expr), String> {
        let tcx = self.tcx;
        let ty = self.monomorphize(destination.ty(&self.mir.local_decls, tcx).ty);
        let ty::Adt(def, args) = ty.kind() else {
            return Err(format!(
                "`#[js(nullable)]` returns an `Option`, and this returns `{ty}`"
            ));
        };
        if !def.is_enum() || !tcx.is_diagnostic_item(rustc_span::sym::Option, def.did()) {
            return Err(format!(
                "`#[js(nullable)]` returns an `Option`, and this returns `{ty}`"
            ));
        }

        let variant = |name: &str| {
            def.variants()
                .iter_enumerated()
                .find(|(_, variant)| variant.name.as_str() == name)
                .map(|(index, _)| index)
                .ok_or_else(|| format!("`{ty}` has no `{name}` variant"))
        };
        let value_ty = self.monomorphize(args.type_at(0));
        let (hold, held) = self.temp(value);
        let some = crate::value::enum_value(
            tcx,
            ty,
            variant("Some")?,
            vec![(
                "_0".to_owned(),
                self.host_boundary_value(value_ty, held.clone()),
            )],
        )
        .ok_or_else(|| format!("`{ty}` has no enum representation"))?;
        let none = crate::value::enum_value(tcx, ty, variant("None")?, Vec::new())
            .ok_or_else(|| format!("`{ty}` has no enum representation"))?;

        Ok((
            hold,
            jsast::cond(
                jsast::binary(jsast::BinOp::Eq, held, jsast::null()),
                none,
                some,
            ),
        ))
    }

    /// Applies representation rules a typed JavaScript result must satisfy before Rust sees it.
    fn host_boundary_value(&self, ty: rustc_middle::ty::Ty<'tcx>, value: Expr) -> Expr {
        match ty.kind() {
            rustc_middle::ty::Float(rustc_middle::ty::FloatTy::F32) => jsast::fround(value),
            _ => value,
        }
    }

    /// The success value declared by a `Result<T, E>` destination.
    fn result_value_ty(&self, destination: mir::Place<'tcx>) -> Option<rustc_middle::ty::Ty<'tcx>> {
        let ty = self.monomorphize(destination.ty(&self.mir.local_decls, self.tcx).ty);
        let rustc_middle::ty::Adt(def, args) = ty.kind() else {
            return None;
        };
        if !self
            .tcx
            .is_diagnostic_item(rustc_span::sym::Result, def.did())
        {
            return None;
        }
        Some(self.monomorphize(args.type_at(0)))
    }
}
