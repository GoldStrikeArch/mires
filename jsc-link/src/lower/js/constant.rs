//! Constants: a [`ConstTree`] as a JavaScript expression at a type, the hoisted-constant names v1
//! shares, the caller `Location` value, and the `static` initializer (v1 `backend/src/constant.rs`
//! and `intrinsics.rs::caller_location_value`, over the decoded tree instead of an allocation).

use rcgjs_object::jir::decline::OpClass;
use rcgjs_object::jir::origin::SourceFact;
use rcgjs_object::jir::semantic::{ConstTree, GlobalReference};
use rcgjs_object::jir::types::{EnumRepr, FieldKey, IntRepr, NominalForm, TypeId, TypeKind};
use rcgjs_object::jsast::{self, Expr};
use rcgjs_object::ptr;

use super::value::{self, TAG};
use super::{decline, JsCx, Res};

/// The byte length from which a `&str` constant is hoisted into a shared `const`
/// (`constant.rs:44`).
pub const HOIST_STRING_BYTES: usize = 32;

/// v1 `naming.rs::hash64`: FNV-1a over the key of a synthetic item.
pub fn hash64(bytes: &str) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in bytes.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

/// The name v1 gives a hoisted constant: `<prefix>$h<16 hex digits of hash64(key)>`
/// (`naming.rs::readable` for a prefix that is already an identifier).
pub fn hoisted_name(prefix: &str, key: &str) -> String {
    format!("{prefix}$h{:016x}", hash64(key))
}

impl<'a> JsCx<'a> {
    /// A `&str` literal: the shared `const` v1 hoisted it into when this item's baseline
    /// references that constant, the inline literal otherwise.
    ///
    /// The producer decided whether to hoist (`js-hoist-consts`) and the item's `deps` say what it
    /// decided: a hoisted string is referenced by name there. Following `deps` rather than the
    /// link-time option is what keeps the lowering's `refs` equal to the baseline's.
    ///
    /// Under `js-str=bytes` the literal is the byte string of `text` (`lower/bstr.rs`). The shared
    /// `const` the producer hoisted holds the UTF-16 text, which is the same string only when the
    /// text is ASCII, so a non-ASCII literal is spelled inline.
    pub fn string_literal(&self, text: &str) -> Expr {
        let bytes = crate::lower::bstr::enabled();
        if text.len() >= HOIST_STRING_BYTES && (!bytes || text.is_ascii()) {
            let name = hoisted_name("s", text);
            if self.summary.deps.iter().any(|dep| dep == &name) {
                return jsast::id(name);
            }
        }
        if bytes {
            return jsast::string(crate::lower::bstr::byte_string(text));
        }
        jsast::string(text)
    }

    /// A [`ConstTree`] as the JavaScript value of type `ty` (v1 `constant.rs`).
    pub fn const_expr(&self, tree: &ConstTree, ty: TypeId) -> Res<Expr> {
        self.const_expr_at(tree, ty, 0)
    }

    /// Lowers a 64-bit integer [`ConstTree`] directly to a signed-`Int32` `(lo, hi)` pair.
    pub fn const_i64_pair(&self, tree: &ConstTree, ty: TypeId) -> Option<(Expr, Expr)> {
        let mut cur_ty = ty;
        for _ in 0..16 {
            let descriptor = self.table.get(cur_ty)?;
            if let Some(inner) = descriptor.js.transparent {
                cur_ty = inner.ty;
                continue;
            }
            if let TypeKind::Pattern(base) = descriptor.kind {
                cur_ty = base;
                continue;
            }
            break;
        }
        if !super::i64_pair::is_i64(self.table, cur_ty) {
            return None;
        }
        match tree {
            ConstTree::Scalar { bits, .. } => Some(super::i64_pair::literal_pair(*bits)),
            _ => None,
        }
    }

    fn const_expr_at(&self, tree: &ConstTree, ty: TypeId, depth: u32) -> Res<Expr> {
        if depth > 64 {
            return Err(decline(
                OpClass::Constant,
                "constant tree deeper than 64 levels",
            ));
        }
        let table = self.table;
        let interns = self.interns;
        let Some(descriptor) = table.get(ty) else {
            return Err(decline(
                OpClass::Constant,
                format!("constant at type {:032x} missing from the type table", ty.0),
            ));
        };
        // A transparent wrapper *is* its field, and the encoder decoded the tree at the field's
        // type (`encode.rs::const_tree`).
        if let Some(inner) = descriptor.js.transparent {
            return self.const_expr_at(tree, inner.ty, depth + 1);
        }
        if let TypeKind::Pattern(base) = descriptor.kind {
            return self.const_expr_at(tree, base, depth + 1);
        }
        match tree {
            ConstTree::Unit => Ok(jsast::undefined()),
            ConstTree::Scalar { bits, .. } => self.scalar_expr(*bits, ty),
            ConstTree::Str(id) => {
                let text = interns.str(*id).unwrap_or("");
                Ok(self.string_literal(text))
            }
            ConstTree::Address(address) => Ok(jsast::num(*address as f64)),
            ConstTree::Global { target, offset } => self.global_expr(target, *offset, ty),
            ConstTree::Aggregate(fields) => self.aggregate_expr(fields, ty, depth),
            ConstTree::Variant { variant, fields } => {
                let variants = table.variants(ty);
                let Some(descriptor) = variants.get(*variant as usize) else {
                    return Err(decline(
                        OpClass::Constant,
                        format!(
                            "constant names variant {variant} of an enum with {} variants",
                            variants.len()
                        ),
                    ));
                };
                let mut entries = Vec::with_capacity(fields.len());
                for (field, tree) in descriptor.fields.iter().zip(fields) {
                    let key = self.field_key_name(field.key)?;
                    entries.push((key, self.const_expr_at(tree, field.ty, depth + 1)?));
                }
                value::enum_value(table, interns, ty, *variant, entries).ok_or_else(|| {
                    decline(
                        OpClass::Constant,
                        "a variant constant at a type that is not an enum",
                    )
                })
            }
            ConstTree::Reference(inner) => {
                let Some(pointee) = value::pointee_of(table, ty) else {
                    return Err(decline(
                        OpClass::Constant,
                        "a reference constant at a type that is not a pointer",
                    ));
                };
                self.reference_expr(inner, ty, pointee, depth)
            }
            ConstTree::Slice(elements) => {
                let Some(pointee) = value::pointee_of(table, ty) else {
                    return Err(decline(
                        OpClass::Constant,
                        "a slice constant at a type that is not a pointer",
                    ));
                };
                self.slice_expr(elements, pointee, depth)
            }
        }
    }

    /// The JS key of a field descriptor, for a struct or variant object.
    fn field_key_name(&self, key: FieldKey) -> Res<String> {
        match key {
            FieldKey::Name(id) => Ok(self.interns.str(id).unwrap_or("").to_owned()),
            FieldKey::Index(index) => Ok(format!("_{index}")),
            FieldKey::Transparent => Err(decline(
                OpClass::Constant,
                "a named field of a zero-sized aggregate has no key in the type table",
            )),
        }
    }

    /// `Scalar { bits }` at `ty` (`constant.rs::scalar_to_expr`): the primitives directly, and
    /// the register-sized aggregates by reading the bits at their type.
    fn scalar_expr(&self, bits: u128, ty: TypeId) -> Res<Expr> {
        let table = self.table;
        let Some(descriptor) = table.get(ty) else {
            return Err(decline(
                OpClass::Constant,
                "scalar constant at an unknown type",
            ));
        };
        match &descriptor.kind {
            TypeKind::Bool => Ok(jsast::boolean(bits != 0)),
            TypeKind::Char => Ok(jsast::num(bits as f64)),
            TypeKind::Integer { .. } => value::int_literal(table, ty, bits)
                .ok_or_else(|| decline(OpClass::Constant, "integer literal at a non-integer type")),
            TypeKind::Float { bits: 32 } => Ok(jsast::num(f32::from_bits(bits as u32) as f64)),
            TypeKind::Float { .. } => Ok(jsast::num(f64::from_bits(bits as u64))),
            // A pointer in a register with no provenance is an address (`constant.rs:400`).
            TypeKind::RawPointer { .. } | TypeKind::Reference { .. } | TypeKind::FnPtr { .. } => {
                Ok(jsast::num(bits as f64))
            }
            // A `Number` enum is its discriminant, re-signed through its repr.
            TypeKind::Adt {
                form: NominalForm::Enum,
                variants,
                niche,
                ..
            } => {
                if let Some(EnumRepr::Number {
                    bits: width,
                    signed,
                }) = descriptor.js.enum_repr
                {
                    let discr = value::resign(signed, width, bits);
                    let repr = if width > 32 {
                        IntRepr::BigInt
                    } else {
                        IntRepr::Number
                    };
                    return Ok(value::signed_literal(repr, discr));
                }
                // A direct tag: the variant whose tag these bits are, fieldless.
                if variants
                    .iter()
                    .all(|variant| variant.physical_tag.is_some())
                {
                    let Some(index) = variants.iter().position(|variant| {
                        variant.physical_tag.is_some_and(|tag| tag.bits == bits)
                    }) else {
                        return Err(decline(
                            OpClass::Constant,
                            format!("scalar constant `{bits}` names no variant of the enum"),
                        ));
                    };
                    if !variants[index]
                        .fields
                        .iter()
                        .all(|field| table.is_zst(field.ty))
                    {
                        return Err(decline(
                            OpClass::Constant,
                            "scalar constant of a tagged enum with a payload",
                        ));
                    }
                    return value::enum_value(table, self.interns, ty, index as u32, Vec::new())
                        .ok_or_else(|| decline(OpClass::Constant, "scalar enum constant"));
                }
                // A niche: the bit pattern names a niche variant or the untagged one, whose one
                // non-zero-sized field holds the bits.
                let Some(niche) = niche else {
                    return Err(decline(
                        OpClass::Constant,
                        "scalar constant of an enum with neither a direct tag nor a niche",
                    ));
                };
                let index = value::niche_variant(table, variants, niche, bits);
                let Some(variant) = variants.get(index as usize) else {
                    return Err(decline(OpClass::Constant, "niche names a missing variant"));
                };
                let mut entries = Vec::new();
                let mut carried = false;
                for field in &variant.fields {
                    let key = self.field_key_name(field.key)?;
                    if table.is_zst(field.ty) {
                        entries.push((key, jsast::undefined()));
                    } else if index != niche.untagged_variant {
                        return Err(decline(
                            OpClass::Constant,
                            "scalar constant of a niche variant with a payload",
                        ));
                    } else if carried {
                        return Err(decline(
                            OpClass::Constant,
                            "scalar constant of a niche enum with two non-zero-sized fields",
                        ));
                    } else {
                        carried = true;
                        entries.push((key, self.scalar_expr(bits, field.ty)?));
                    }
                }
                value::enum_value(table, self.interns, ty, index, entries)
                    .ok_or_else(|| decline(OpClass::Constant, "scalar enum constant"))
            }
            // A struct or tuple small enough for const eval to keep in a register: exactly one
            // field is non-zero-sized and holds the bits; the rest are `undefined`
            // (`constant.rs::read_alloc` over a scratch allocation).
            TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } => {
                let Some(variant) = variants.first() else {
                    return Ok(jsast::object(Vec::new()));
                };
                let mut entries = Vec::with_capacity(variant.fields.len());
                let mut carried = false;
                for field in &variant.fields {
                    if matches!(field.key, FieldKey::Transparent) {
                        continue;
                    }
                    let key = self.field_key_name(field.key)?;
                    if table.is_zst(field.ty) {
                        entries.push((key, jsast::undefined()));
                    } else if carried {
                        return Err(decline(
                            OpClass::Constant,
                            "scalar constant of a struct with two non-zero-sized fields",
                        ));
                    } else {
                        carried = true;
                        entries.push((key, self.scalar_expr(bits, field.ty)?));
                    }
                }
                Ok(jsast::object(entries))
            }
            TypeKind::Tuple(elements)
            | TypeKind::Closure {
                upvars: elements, ..
            } => {
                if elements.is_empty() {
                    return Ok(jsast::undefined());
                }
                let mut items = Vec::with_capacity(elements.len());
                let mut carried = false;
                for element in elements {
                    if table.is_zst(*element) {
                        items.push(jsast::undefined());
                    } else if carried {
                        return Err(decline(
                            OpClass::Constant,
                            "scalar constant of a tuple or closure with two non-zero-sized elements",
                        ));
                    } else {
                        carried = true;
                        items.push(self.scalar_expr(bits, *element)?);
                    }
                }
                Ok(jsast::array(items))
            }
            other => Err(decline(
                OpClass::Constant,
                format!("scalar constant at a `{other:?}` type is not decodable at link time"),
            )),
        }
    }

    /// `Aggregate(fields)` at `ty`: a struct object, a tuple/array/closure array.
    fn aggregate_expr(&self, fields: &[ConstTree], ty: TypeId, depth: u32) -> Res<Expr> {
        let table = self.table;
        let Some(descriptor) = table.get(ty) else {
            return Err(decline(
                OpClass::Constant,
                "aggregate constant at an unknown type",
            ));
        };
        match &descriptor.kind {
            TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } => {
                let Some(variant) = variants.first() else {
                    return Ok(jsast::object(Vec::new()));
                };
                let mut entries = Vec::with_capacity(fields.len());
                for (field, tree) in variant.fields.iter().zip(fields) {
                    // A zero-sized struct's fields have no keys in the table; the value is `{}`
                    // (spec: `constant.rs:255-269` prints the keys, with `undefined` values).
                    if matches!(field.key, FieldKey::Transparent) {
                        continue;
                    }
                    let key = self.field_key_name(field.key)?;
                    entries.push((key, self.const_expr_at(tree, field.ty, depth + 1)?));
                }
                Ok(jsast::object(entries))
            }
            TypeKind::Adt {
                form: NominalForm::Union,
                ..
            } => Err(decline(
                OpClass::Constant,
                "a union constant that is not transparent",
            )),
            TypeKind::Tuple(elements) => {
                if elements.is_empty() {
                    return Ok(jsast::undefined());
                }
                let items = elements
                    .iter()
                    .zip(fields)
                    .map(|(element, tree)| self.const_expr_at(tree, *element, depth + 1))
                    .collect::<Res<Vec<_>>>()?;
                Ok(jsast::array(items))
            }
            TypeKind::Array { element, .. } => {
                let items = fields
                    .iter()
                    .map(|tree| self.const_expr_at(tree, *element, depth + 1))
                    .collect::<Res<Vec<_>>>()?;
                Ok(jsast::array(items))
            }
            TypeKind::Closure { upvars, .. } => {
                let items = upvars
                    .iter()
                    .zip(fields)
                    .map(|(upvar, tree)| self.const_expr_at(tree, *upvar, depth + 1))
                    .collect::<Res<Vec<_>>>()?;
                Ok(jsast::array(items))
            }
            other => Err(decline(
                OpClass::Constant,
                format!("aggregate constant at a `{other:?}` type"),
            )),
        }
    }

    /// A pointer into anonymous memory (`constant.rs::pointer_expr`, `GlobalAlloc::Memory`).
    fn reference_expr(
        &self,
        inner: &ConstTree,
        ty: TypeId,
        pointee: TypeId,
        depth: u32,
    ) -> Res<Expr> {
        let table = self.table;
        match inner {
            ConstTree::Str(id) if value::is_str(table, pointee) => {
                Ok(self.string_literal(self.interns.str(*id).unwrap_or("")))
            }
            ConstTree::Slice(elements) => self.slice_expr(elements, pointee, depth),
            _ => {
                let value = self.const_expr_at(inner, pointee, depth + 1)?;
                if table.is_indirect(pointee) {
                    Ok(self.boxed_if_raw(ty, pointee, value))
                } else {
                    // A reference to a primitive is a slot into a fresh single element array.
                    Ok(ptr::slot(jsast::array(vec![value]), jsast::num(0)))
                }
            }
        }
    }

    /// `__rt.box(value)` for a raw pointer to a slot pointee, the value itself otherwise
    /// (`constant.rs::boxed_if_raw`).
    fn boxed_if_raw(&self, ty: TypeId, pointee: TypeId, value: Expr) -> Expr {
        if value::is_raw(self.table, ty) && value::is_slot_pointee(self.table, pointee) {
            return jsast::rt_call("box", vec![value]);
        }
        value
    }

    /// `{ buf: [..], off: 0, len }` over the decoded elements (`constant.rs::slice_expr`).
    fn slice_expr(&self, elements: &[ConstTree], pointee: TypeId, depth: u32) -> Res<Expr> {
        let table = self.table;
        let Some(TypeKind::Slice(element)) = value::kind(table, pointee) else {
            return Err(decline(
                OpClass::Constant,
                "a slice constant whose pointee is not a slice",
            ));
        };
        let element = *element;
        let items = elements
            .iter()
            .map(|tree| self.const_expr_at(tree, element, depth + 1))
            .collect::<Res<Vec<_>>>()?;
        let length = items.len();
        Ok(ptr::fat_slice(
            jsast::array(items),
            jsast::num(0),
            jsast::num(length as f64),
        ))
    }

    /// A pointer to a global (`constant.rs::pointer_expr`, `Function`/`Static`).
    fn global_expr(&self, target: &GlobalReference, offset: u64, ty: TypeId) -> Res<Expr> {
        match target {
            GlobalReference::Function { link_name, .. } => Ok(jsast::id(
                self.interns.str(*link_name).unwrap_or("").to_owned(),
            )),
            // A reference to a `static` is the module level binding it was emitted as. For an
            // indirect type the binding's own value *is* the reference; a primitive has no
            // identity to point at, so such a `static` is emitted **boxed** (`let S = [value]`,
            // `function.rs::lower_static`) and the reference is the slot naming its one element.
            GlobalReference::Static { link_name, .. } => {
                if offset != 0 {
                    return Err(decline(
                        OpClass::Constant,
                        "a pointer into the middle of a `static` is not supported by \
                         rustc_codegen_js",
                    ));
                }
                let binding = jsast::id(self.interns.str(*link_name).unwrap_or("").to_owned());
                let Some(pointee) = value::pointee_of(self.table, ty) else {
                    return Err(decline(
                        OpClass::Constant,
                        "a reference to a `static` at a type that is not a pointer",
                    ));
                };
                if !self.table.is_indirect(pointee) {
                    return Ok(ptr::slot(binding, jsast::num(0)));
                }
                // `boxed_if_raw`: a raw pointer to a slot pointee is a heap block handle.
                if value::is_raw(self.table, ty) && value::is_slot_pointee(self.table, pointee) {
                    return Ok(jsast::rt_call("box", vec![binding]));
                }
                Ok(binding)
            }
            GlobalReference::Type(_) => Err(decline(
                OpClass::Constant,
                format!("a pointer to a type allocation at type {:032x}", ty.0),
            )),
        }
    }

    /// The caller `Location` of `fact`, as v1 `intrinsics.rs::caller_location_value` spells it:
    /// the `panic_location` lang item's struct with its first three fields `file`, `line`,
    /// `col + 1`, hoisted under `loc$h..` when this item's baseline references that constant.
    pub fn caller_location_value(&self, fact: &SourceFact) -> Res<Expr> {
        let file = self.interns.str(fact.file).unwrap_or("").to_owned();
        let line = fact.line;
        let column = fact.col + 1;
        let hoisted = hoisted_name("loc", &format!("loc\u{1}{file}\u{1}{line}\u{1}{column}"));
        // `Location::file` is a Rust `str`: a byte string under `js-str=bytes`, which the
        // producer's hoisted constant is only when the path is ASCII.
        let bytes = crate::lower::bstr::enabled();
        if (!bytes || file.is_ascii()) && self.summary.deps.iter().any(|dep| dep == &hoisted) {
            return Ok(jsast::id(hoisted));
        }
        let file = if bytes {
            crate::lower::bstr::byte_string(&file)
        } else {
            file
        };
        let (file_key, line_key, col_key) = self.location_field_keys()?;
        Ok(jsast::object(vec![
            (file_key, jsast::string(file)),
            (line_key, jsast::num(line as f64)),
            (col_key, jsast::num(column as f64)),
        ]))
    }

    /// The field keys of the `panic_location` lang item's struct: the encoder describes its
    /// reference type in the table whenever it records a `track_caller` fact; here it is found
    /// by shape (a struct whose first field points at `str` and whose next two are integers).
    fn location_field_keys(&self) -> Res<(String, String, String)> {
        let table = self.table;
        // Every struct shaped like a location, with its name.
        let mut candidates: Vec<(&str, (String, String, String))> = Vec::new();
        for descriptor in table.descriptors() {
            let TypeKind::Adt {
                form: NominalForm::Struct,
                variants,
                ..
            } = &descriptor.kind
            else {
                continue;
            };
            let Some(variant) = variants.first() else {
                continue;
            };
            if variant.fields.len() < 3 {
                continue;
            }
            let file_points_at_str = value::pointee_of(table, variant.fields[0].ty)
                .is_some_and(|pointee| value::is_str(table, pointee));
            let ints = table.int_info(variant.fields[1].ty).is_some()
                && table.int_info(variant.fields[2].ty).is_some();
            if !file_points_at_str || !ints {
                continue;
            }
            let keys = (
                self.field_key_name(variant.fields[0].key)?,
                self.field_key_name(variant.fields[1].key)?,
                self.field_key_name(variant.fields[2].key)?,
            );
            candidates.push((self.interns.str(variant.name).unwrap_or(""), keys));
        }
        // The lang item is literally `Location`; a program has exactly one.
        let named: Vec<&(String, String, String)> = candidates
            .iter()
            .filter(|(name, _)| *name == "Location")
            .map(|(_, keys)| keys)
            .collect();
        if let [keys] = named.as_slice() {
            return Ok((*keys).clone());
        }
        let mut keys = candidates.iter().map(|(_, keys)| keys);
        let Some(first) = keys.next() else {
            return Err(decline(
                OpClass::Constant,
                "a caller location is needed, but the type table describes no `Location` struct",
            ));
        };
        if keys.all(|other| other == first) {
            return Ok(first.clone());
        }
        Err(decline(
            OpClass::Constant,
            "the caller `Location` type is ambiguous in the type table",
        ))
    }
}

/// The tag key, re-exported for the constant tests.
#[allow(dead_code)]
pub const TAG_KEY: &str = TAG;
