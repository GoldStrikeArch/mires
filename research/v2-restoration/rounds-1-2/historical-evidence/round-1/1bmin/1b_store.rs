//! Whole-value stores of aggregates, written through into the storage object a place already
//! holds (CONTRACT.md, "Storage identity"). `jsc-link`'s `lower::js::store` is the same rule over
//! the type table; read that module's documentation for the why.
//!
//! The new value arrives as the expression the rvalue lowered to. When that expression is an
//! object or array literal -- a fresh aggregate, or the field-by-field copy `clone_expr` spells --
//! its entries are written straight into the existing object and the literal itself is never
//! built. Otherwise the value is evaluated once into a temporary and its leaves are read from it.

use rustc_middle::ty::{self, Ty, TyCtxt};

use crate::base::FnCx;
use crate::jsast::{self, Expr, Stmt};
use crate::ptr::{owns_object, product_fields, write_shape};
use crate::value::{field_key, peel_pattern, peel_transparent, EnumRepr, FieldKey, TAG};

/// How many leaf assignments a field-wise store spells before it calls the runtime instead.
const MAX_LEAVES: usize = 8;
/// How deep a field-wise store descends into nested aggregates.
const MAX_DEPTH: u32 = 6;

#[derive(Clone)]
enum Step {
    Name(String),
    Index(usize),
}

/// Where a leaf's new value comes from.
#[derive(Clone)]
enum From {
    /// A path into root `n` (an expression evaluated once, `Plan::roots`).
    Root(usize, Vec<Step>),
    /// An expression of its own: an entry of a literal, or a literal `undefined`.
    Expr(Expr),
}

enum Leaf {
    Set(Vec<Step>, From),
    Overwrite(Vec<Step>, From),
}

struct Plan {
    roots: Vec<Expr>,
    leaves: Vec<Leaf>,
}

fn at(base: Expr, path: &[Step]) -> Expr {
    path.iter().fold(base, |base, step| match step {
        Step::Name(name) => jsast::member(base, name.clone()),
        Step::Index(index) => jsast::index(base, jsast::num(*index as f64)),
    })
}

fn joined(path: &[Step], step: Step) -> Vec<Step> {
    let mut path = path.to_vec();
    path.push(step);
    path
}

fn atomic(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Ident(_)
            | Expr::Num(_)
            | Expr::RawNum(_)
            | Expr::BigInt(_)
            | Expr::Str(_)
            | Expr::Bool(_)
            | Expr::Undefined
            | Expr::Null
    )
}

/// The entry of an object literal under `key`.
fn entry<'e>(entries: &'e [(String, Expr)], key: &str) -> Option<&'e Expr> {
    entries
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, expr)| expr)
}

impl Plan {
    /// A source to read leaves of `ty` from: a literal is taken apart, anything else becomes a
    /// root evaluated once.
    fn source(&mut self, expr: Expr) -> From {
        match expr {
            Expr::Object(_) | Expr::Array(_) => From::Expr(expr),
            expr => {
                self.roots.push(expr);
                From::Root(self.roots.len() - 1, Vec::new())
            }
        }
    }

    /// The source of one field of `from`, or `None` when the literal lacks it.
    fn field(&mut self, from: &From, step: &Step) -> Option<From> {
        Some(match from {
            From::Root(root, path) => From::Root(*root, joined(path, step.clone())),
            From::Expr(Expr::Object(entries)) => match step {
                Step::Name(name) => From::Expr(entry(entries, name)?.clone()),
                Step::Index(_) => return None,
            },
            From::Expr(Expr::Array(items)) => match step {
                Step::Index(index) => From::Expr(items.get(*index)?.clone()),
                Step::Name(_) => return None,
            },
            From::Expr(_) => return None,
        })
    }

    fn plan<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        ty: Ty<'tcx>,
        from: From,
        path: &[Step],
        depth: u32,
    ) -> bool {
        if depth > MAX_DEPTH || self.leaves.len() > MAX_LEAVES {
            return false;
        }
        let ty = peel_pattern(peel_transparent(tcx, ty));
        if crate::value::is_zst(tcx, ty) {
            return true;
        }
        // A non-literal value of a shape a field-wise copy reads: evaluate it once.
        let from = match from {
            From::Expr(expr) if !matches!(expr, Expr::Object(_) | Expr::Array(_)) => {
                self.source(expr)
            }
            from => from,
        };
        match ty.kind() {
            ty::Adt(definition, _) if definition.is_struct() => {
                self.product(tcx, ty, from, path, depth)
            }
            ty::Tuple(_) | ty::Closure(..) => self.product(tcx, ty, from, path, depth),
            ty::Adt(definition, _)
                if definition.is_enum()
                    && matches!(
                        crate::value::enum_repr(tcx, ty),
                        Some(EnumRepr::Tag { fieldless: false })
                    ) =>
            {
                // The tag and every payload key any variant uses; the keys the new variant does
                // not use are cleared.
                let mut keys: Vec<String> = Vec::new();
                for (variant, def) in definition.variants().iter_enumerated() {
                    for (index, _) in def.fields.iter_enumerated() {
                        let Some(FieldKey::Name(key)) = field_key(tcx, ty, Some(variant), index)
                        else {
                            return false;
                        };
                        if !keys.contains(&key) {
                            keys.push(key);
                        }
                    }
                }
                for key in std::iter::once(TAG.to_owned()).chain(keys) {
                    let step = Step::Name(key);
                    let value = match &from {
                        From::Expr(Expr::Object(entries)) => {
                            let Step::Name(key) = &step else { return false };
                            From::Expr(entry(entries, key).cloned().unwrap_or(Expr::Undefined))
                        }
                        other => match self.field(other, &step) {
                            Some(value) => value,
                            None => return false,
                        },
                    };
                    self.leaves.push(Leaf::Set(joined(path, step), value));
                }
                self.leaves.len() <= MAX_LEAVES
            }
            ty::Array(element, count) => match &from {
                // Elements that are aggregates keep their objects: each is written into, which
                // the runtime does (`_deep`) for an array too long to spell here.
                From::Root(..) if owns_object(tcx, *element) => {
                    let Some(length) = count.try_to_target_usize(tcx) else {
                        return false;
                    };
                    for index in 0..length as usize {
                        let step = Step::Index(index);
                        let Some(value) = self.field(&from, &step) else {
                            return false;
                        };
                        if !self.plan_field(tcx, *element, value, joined(path, step), depth) {
                            return false;
                        }
                    }
                    true
                }
                From::Expr(Expr::Array(items)) => {
                    for index in 0..items.len() {
                        let step = Step::Index(index);
                        let Some(value) = self.field(&from, &step) else {
                            return false;
                        };
                        if !self.plan_field(tcx, *element, value, joined(path, step), depth) {
                            return false;
                        }
                    }
                    true
                }
                _ => {
                    self.leaves.push(Leaf::Overwrite(path.to_vec(), from));
                    true
                }
            },
            _ => false,
        }
    }

    fn product<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        ty: Ty<'tcx>,
        from: From,
        path: &[Step],
        depth: u32,
    ) -> bool {
        for (key, field_ty, _) in product_fields(tcx, ty) {
            if crate::value::is_zst(tcx, field_ty) {
                continue;
            }
            let step = match key {
                FieldKey::Name(name) => Step::Name(name),
                FieldKey::Index(index) => Step::Index(index),
                FieldKey::Transparent => return false,
            };
            let Some(value) = self.field(&from, &step) else {
                return false;
            };
            if !self.plan_field(tcx, field_ty, value, joined(path, step), depth) {
                return false;
            }
        }
        true
    }

    fn plan_field<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        field_ty: Ty<'tcx>,
        from: From,
        path: Vec<Step>,
        depth: u32,
    ) -> bool {
        if !owns_object(tcx, field_ty) {
            self.leaves.push(Leaf::Set(path, from));
            return self.leaves.len() <= MAX_LEAVES;
        }
        let peeled = peel_pattern(peel_transparent(tcx, field_ty));
        let descends = match peeled.kind() {
            ty::Adt(definition, _) if definition.is_struct() => true,
            ty::Tuple(_) | ty::Closure(..) | ty::Array(..) => true,
            ty::Adt(definition, _) if definition.is_enum() => matches!(
                crate::value::enum_repr(tcx, peeled),
                Some(EnumRepr::Tag { fieldless: false })
            ),
            _ => false,
        };
        if descends {
            return self.plan(tcx, field_ty, from, &path, depth + 1);
        }
        self.leaves.push(Leaf::Overwrite(path, from));
        self.leaves.len() <= MAX_LEAVES
    }
}

/// The statements that store `value`, a whole aggregate of type `ty`, into `dst`, an expression
/// naming the storage object the place already holds, written through.
pub(crate) fn store_into<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    ty: Ty<'tcx>,
    dst: Expr,
    value: Expr,
) -> Vec<Stmt> {
    let tcx = fx.tcx;
    if crate::value::is_zst(tcx, ty) {
        return vec![jsast::expr_stmt(value)];
    }
    let mut plan = Plan {
        roots: Vec::new(),
        leaves: Vec::new(),
    };
    let from = plan.source(value.clone());
    if !plan.plan(tcx, ty, from, &[], 0) || plan.leaves.len() > MAX_LEAVES {
        return vec![jsast::expr_stmt(runtime_store(tcx, ty, dst, value))];
    }
    let mut out = Vec::new();
    let roots: Vec<Expr> = plan
        .roots
        .into_iter()
        .map(|root| {
            if atomic(&root) {
                root
            } else {
                let (stmt, name) = fx.temp(root);
                out.push(stmt);
                name
            }
        })
        .collect();
    let mut values = Vec::with_capacity(plan.leaves.len());
    for leaf in &plan.leaves {
        let (Leaf::Set(_, from) | Leaf::Overwrite(_, from)) = leaf;
        values.push(match from {
            From::Root(root, path) => at(roots[*root].clone(), path),
            From::Expr(expr) if atomic(expr) => expr.clone(),
            From::Expr(expr) => {
                let (stmt, name) = fx.temp(expr.clone());
                out.push(stmt);
                name
            }
        });
    }
    let dst = if atomic(&dst) || plan.leaves.len() <= 1 {
        dst
    } else {
        let (stmt, name) = fx.temp(dst);
        out.push(stmt);
        name
    };
    for (leaf, value) in plan.leaves.iter().zip(values) {
        out.push(match leaf {
            Leaf::Set(path, _) => jsast::assign_stmt(at(dst.clone(), path), value),
            Leaf::Overwrite(path, _) => jsast::expr_stmt(jsast::rt_call(
                "overwrite",
                vec![at(dst.clone(), path), value],
            )),
        });
    }
    out
}

/// The runtime spelling of a whole-value store the field-wise one gave up on.
pub(crate) fn runtime_store<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, dst: Expr, value: Expr) -> Expr {
    match write_shape(tcx, ty) {
        Some(shape) => jsast::rt_call("_deep", vec![dst, value, jsast::string(shape)]),
        None => jsast::rt_call("overwrite", vec![dst, value]),
    }
}

/// The statements that exchange the contents of the storage objects `x` and `y` of type `ty`
/// leaf by leaf, or `None` when the type has no field-wise spelling.
pub(crate) fn swap_into<'tcx>(
    fx: &FnCx<'_, 'tcx>,
    ty: Ty<'tcx>,
    x: Expr,
    y: Expr,
) -> Option<Vec<Stmt>> {
    let mut plan = Plan {
        roots: Vec::new(),
        leaves: Vec::new(),
    };
    let from = plan.source(jsast::undefined());
    if !plan.plan(fx.tcx, ty, from, &[], 0)
        || plan.leaves.len() > MAX_LEAVES
        || plan
            .leaves
            .iter()
            .any(|leaf| matches!(leaf, Leaf::Overwrite(..)))
    {
        return None;
    }
    let mut out = Vec::new();
    let name = |expr: Expr, out: &mut Vec<Stmt>| {
        if atomic(&expr) {
            expr
        } else {
            let (stmt, name) = fx.temp(expr);
            out.push(stmt);
            name
        }
    };
    let x = name(x, &mut out);
    let y = name(y, &mut out);
    let (_, spare) = fx.temp(jsast::undefined());
    for leaf in &plan.leaves {
        let Leaf::Set(path, _) = leaf else {
            return None;
        };
        out.push(jsast::assign_stmt(spare.clone(), at(x.clone(), path)));
        out.push(jsast::assign_stmt(at(x.clone(), path), at(y.clone(), path)));
        out.push(jsast::assign_stmt(at(y.clone(), path), spare.clone()));
    }
    Some(out)
}
