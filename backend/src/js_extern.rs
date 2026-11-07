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
    steps
        .iter()
        .fold(root, |object, step| jsast::member(object, (*step).to_owned()))
}

/// The descriptor `def_id` carries, or `None` for anything that is not a declaration.
pub(crate) fn extern_section(tcx: TyCtxt<'_>, def_id: rustc_hir::def_id::DefId) -> Option<String> {
    let section = tcx.codegen_fn_attrs(def_id).link_section?;
    section.as_str().strip_prefix(PREFIX).map(str::to_owned)
}

impl<'tcx> FnCx<'_, 'tcx> {
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

        let lowered: Vec<Expr> = args
            .iter()
            .map(|arg| self.at(arg.span, || self.codegen_operand(&arg.node)))
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
                Some((last, before)) => jsast::method_call(
                    walk(root, before),
                    (*last).to_owned(),
                    passed.to_vec(),
                ),
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
        };

        if !descriptor.nullable {
            return self.write_place(destination, value);
        }
        match self.nullable(destination, value) {
            Ok((hold, wrapped)) => {
                let mut stmts = vec![hold];
                stmts.extend(self.write_place(destination, wrapped));
                stmts
            }
            Err(why) => vec![jsast::expr_stmt(self.zombie(why))],
        }
    }

    /// What the path is rooted at when it is not rooted at an argument: the module's imported
    /// binding, or a global of the same name.
    ///
    /// Only the path's FIRST segment names it either way. `Chart.register` imports `Chart` and
    /// reaches `register` through it, because `import { Chart.register }` is not a thing anyone
    /// can write.
    fn js_binding(&self, descriptor: &Descriptor) -> Expr {
        let head = descriptor.segments().first().copied().unwrap_or_default().to_owned();
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
                Linkage { fixed_name: Some(local.clone()), root: false, exported: false },
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
    fn nullable(
        &self,
        destination: mir::Place<'tcx>,
        value: Expr,
    ) -> Result<(Stmt, Expr), String> {
        let tcx = self.tcx;
        let ty = self.monomorphize(destination.ty(&self.mir.local_decls, tcx).ty);
        let ty::Adt(def, _) = ty.kind() else {
            return Err(format!("`#[js(nullable)]` returns an `Option`, and this returns `{ty}`"));
        };
        if !def.is_enum() || !tcx.is_diagnostic_item(rustc_span::sym::Option, def.did()) {
            return Err(format!("`#[js(nullable)]` returns an `Option`, and this returns `{ty}`"));
        }

        let variant = |name: &str| {
            def.variants()
                .iter_enumerated()
                .find(|(_, variant)| variant.name.as_str() == name)
                .map(|(index, _)| index)
                .ok_or_else(|| format!("`{ty}` has no `{name}` variant"))
        };
        let (hold, held) = self.temp(value);
        let some = crate::value::enum_value(
            tcx,
            ty,
            variant("Some")?,
            vec![("_0".to_owned(), held.clone())],
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
}
