//! Physical ABI shapes ([`FlatParam`], [`FlatResult`], [`FlatSig`]) and shape discovery/validation.

use crate::jir::repr::{Func, Op, OpKind, ValueId};
use crate::jir::semantic::{Callee, ConstTree, TypeId};
use crate::jir::types::{
    EnumRepr, FieldKey, NominalAbi, NominalForm, PinnedBoundary, PointerForm, TypeKind, TypeTable,
};
use crate::opt::candidates;
use crate::plan::physical_cost::HashFeed;

/// Read-only `js-str=bytes` specialization mode for an immutable `&[u8]` parameter.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ByteStrParam {
    /// Whole `&str`-backed `&[u8]` passed as 1 JS parameter `p` (a Latin-1 byte string with
    /// `off === 0` and `len === p.length`).
    Full,
    /// Subslice of a `&str`-backed `&[u8]` passed as 3 flat JS parameters `(p$buf, p$off, p$len)`
    /// where `p$buf` is a Latin-1 byte string.
    Sub,
}

impl ByteStrParam {
    /// Lattice join: `Full ⊔ Full = Full`, any `Sub` yields `Sub`.
    pub(crate) fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Full, Self::Full) => Self::Full,
            _ => Self::Sub,
        }
    }

    pub(crate) fn hash_into(self, feed: &mut HashFeed) {
        feed.u8(match self {
            Self::Full => 1,
            Self::Sub => 2,
        });
    }
}

/// How an aggregate flat parameter is passed across the Rust boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParamAccess {
    /// Passed by value (`T`).
    ByValue,
    /// Passed by shared reference (`&T`): read-only, no copy-back.
    SharedRef,
    /// Passed by mutable reference (`&mut T`): copies fields back at every `Return` iff `written`.
    MutRef { written: bool },
}

impl ParamAccess {
    pub(crate) fn hash_into(self, feed: &mut HashFeed) {
        match self {
            Self::ByValue => feed.u8(0),
            Self::SharedRef => feed.u8(1),
            Self::MutRef { written } => {
                feed.u8(2);
                feed.bool(written);
            }
        }
    }
}

/// Exclusive physical representation of a flattened parameter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlatParamKind {
    /// A fixed-shape aggregate (`struct`, tuple, array) expanded at JIR level into its non-ZST
    /// scalar fields.
    Aggregate {
        /// The field types, in **declaration** order — the shape the callee rebuilds.
        field_tys: Vec<TypeId>,
        /// The fields that become JavaScript parameters, by index into `field_tys`.
        passed: Vec<u32>,
        access: ParamAccess,
    },
    /// A 1-lane scalar reference (`&Scalar` / `&mut Scalar`) passed by value (and copied back iff
    /// `mutable && written`).
    ScalarRef { mutable: bool, written: bool },
    /// A `&mut Enum` (`EnumRepr::Tag | IntTag`, 2..=4 variants) passed as `(tag, payload_lanes...)`
    /// and copied back at `Return` iff `written`.
    MutEnum {
        field_tys: Vec<TypeId>,
        passed: Vec<u32>,
        written: bool,
    },
    /// A slice pointer carried as physical `(buf, off, len)` lanes, or `(buf, len)` when
    /// `zero_offset` is proven whole-program (`off === 0`). Remains one semantic JIR parameter
    /// until Phase C.
    Slice {
        field_tys: Vec<TypeId>,
        zero_offset: bool,
    },
    /// Read-only `js-str=bytes` specialization for an immutable `&[u8]` parameter (mutually
    /// exclusive with canonical `Slice`). Remains one semantic JIR parameter until Phase C.
    ByteStr {
        mode: ByteStrParam,
        usize_ty: TypeId,
    },
    /// A 64-bit integer (`i64`/`u64`) traveling as two signed-`Int32` `(lo, hi)` lanes. Remains
    /// one semantic JIR parameter until Phase C.
    I64Pair,
    /// A two-variant enum carried as `tag, payload...`. Remains one semantic JIR parameter until
    /// Phase C.
    Tagged(TaggedShape),
    /// A nested fixed product carried as recursively flattened scalar/pointer lanes. Remains one
    /// semantic JIR parameter until Phase C.
    Recursive(RecursiveShape),
}

impl FlatParamKind {
    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        match self {
            Self::Aggregate {
                field_tys,
                passed,
                access,
            } => {
                feed.u8(1);
                feed.slice(field_tys, |f, &ty| f.type_id(ty));
                feed.slice(passed, |f, &idx| f.u32(idx));
                access.hash_into(feed);
            }
            Self::ScalarRef { mutable, written } => {
                feed.u8(2);
                feed.bool(*mutable);
                feed.bool(*written);
            }
            Self::MutEnum {
                field_tys,
                passed,
                written,
            } => {
                feed.u8(3);
                feed.slice(field_tys, |f, &ty| f.type_id(ty));
                feed.slice(passed, |f, &idx| f.u32(idx));
                feed.bool(*written);
            }
            Self::Slice {
                field_tys,
                zero_offset,
            } => {
                feed.u8(4);
                feed.slice(field_tys, |f, &ty| f.type_id(ty));
                feed.bool(*zero_offset);
            }
            Self::ByteStr { mode, usize_ty } => {
                feed.u8(5);
                mode.hash_into(feed);
                feed.type_id(*usize_ty);
            }
            Self::I64Pair => feed.u8(6),
            Self::Tagged(shape) => {
                feed.u8(7);
                shape.hash_into(feed);
            }
            Self::Recursive(shape) => {
                feed.u8(8);
                shape.hash_into(feed);
            }
        }
    }
}

/// One parameter the plan flattens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlatParam {
    /// The aggregate/scalar/enum type the parameter carries, or that a reference parameter points
    /// at (or the slice reference type itself for `Slice`/`ByteStr`).
    pub ty: TypeId,
    /// Exclusive physical representation for this parameter.
    pub kind: FlatParamKind,
}

impl FlatParam {
    /// Builds a flat `i64`/`u64` `(lo, hi)` pair parameter.
    pub fn i64_pair(ty: TypeId) -> Self {
        Self {
            ty,
            kind: FlatParamKind::I64Pair,
        }
    }

    /// Builds a physical read-only byte-string `&[u8]` parameter.
    pub(crate) fn byte_str_param(ty: TypeId, usize_ty: TypeId, mode: ByteStrParam) -> Self {
        Self {
            ty,
            kind: FlatParamKind::ByteStr { mode, usize_ty },
        }
    }

    pub(crate) fn aggregate(
        ty: TypeId,
        field_tys: Vec<TypeId>,
        passed: Vec<u32>,
        access: ParamAccess,
    ) -> Self {
        Self {
            ty,
            kind: FlatParamKind::Aggregate {
                field_tys,
                passed,
                access,
            },
        }
    }

    pub(crate) fn scalar_ref(ty: TypeId, mutable: bool, written: bool) -> Self {
        Self {
            ty,
            kind: FlatParamKind::ScalarRef {
                mutable,
                written: mutable && written,
            },
        }
    }

    pub(crate) fn mut_enum(
        ty: TypeId,
        field_tys: Vec<TypeId>,
        passed: Vec<u32>,
        written: bool,
    ) -> Self {
        Self {
            ty,
            kind: FlatParamKind::MutEnum {
                field_tys,
                passed,
                written,
            },
        }
    }

    pub(crate) fn slice(ty: TypeId, field_tys: Vec<TypeId>) -> Self {
        Self {
            ty,
            kind: FlatParamKind::Slice {
                field_tys,
                zero_offset: false,
            },
        }
    }

    pub(crate) fn tagged_param(ty: TypeId, shape: TaggedShape) -> Self {
        Self {
            ty,
            kind: FlatParamKind::Tagged(shape),
        }
    }

    pub(crate) fn recursive_param(ty: TypeId, shape: RecursiveShape) -> Self {
        Self {
            ty,
            kind: FlatParamKind::Recursive(shape),
        }
    }

    /// Whether the Rust parameter is passed by reference (`&T` / `&mut T`).
    pub fn by_ref(&self) -> bool {
        matches!(
            self.kind,
            FlatParamKind::Aggregate {
                access: ParamAccess::SharedRef | ParamAccess::MutRef { .. },
                ..
            } | FlatParamKind::ScalarRef { .. }
                | FlatParamKind::MutEnum { .. }
        )
    }

    /// Whether this parameter is a mutable reference (`&mut T`).
    pub fn mutable(&self) -> bool {
        matches!(
            self.kind,
            FlatParamKind::Aggregate {
                access: ParamAccess::MutRef { .. },
                ..
            } | FlatParamKind::ScalarRef { mutable: true, .. }
                | FlatParamKind::MutEnum { .. }
        )
    }

    /// Whether the callee writes through this mutable reference and therefore copies fields back
    /// at every `Return`.
    #[cfg(test)]
    pub fn written(&self) -> bool {
        matches!(
            self.kind,
            FlatParamKind::Aggregate {
                access: ParamAccess::MutRef { written: true },
                ..
            } | FlatParamKind::ScalarRef {
                mutable: true,
                written: true,
            } | FlatParamKind::MutEnum { written: true, .. }
        )
    }

    /// Updates whether a mutable reference parameter is written by the callee.
    pub(crate) fn set_written(&mut self, value: bool) {
        match &mut self.kind {
            FlatParamKind::Aggregate {
                access: ParamAccess::MutRef { written },
                ..
            }
            | FlatParamKind::ScalarRef {
                mutable: true,
                written,
            }
            | FlatParamKind::MutEnum { written, .. } => {
                *written = value;
            }
            _ => {}
        }
    }

    /// Whether this parameter is a flattened 1-lane scalar reference (`&T` / `&mut T`).
    pub fn is_scalar_ref(&self) -> bool {
        matches!(self.kind, FlatParamKind::ScalarRef { .. })
    }

    /// Whether this parameter is a flattened `&mut Enum`.
    pub fn is_mut_enum(&self) -> bool {
        matches!(self.kind, FlatParamKind::MutEnum { .. })
    }

    /// Whether this parameter is a flattened slice (`(buf, off, len)` or `(buf, len)`).
    pub fn is_slice(&self) -> bool {
        matches!(self.kind, FlatParamKind::Slice { .. })
    }

    /// Whether this parameter is a zero-offset specialized slice (`off === 0`), carried as 2 JS
    /// parameters `(buf, len)` with `off` reconstructed as constant `0` in the callee.
    pub fn is_off_zero_slice(&self) -> bool {
        matches!(
            self.kind,
            FlatParamKind::Slice {
                zero_offset: true,
                ..
            }
        )
    }

    /// Marks this slice parameter as zero-offset (`off === 0`). Returns `true` if transitioned.
    pub(crate) fn set_off_zero_slice(&mut self) -> bool {
        if let FlatParamKind::Slice { zero_offset, .. } = &mut self.kind {
            if !*zero_offset {
                *zero_offset = true;
                return true;
            }
        }
        false
    }

    /// The read-only byte-string mode when this is a `ByteStr` parameter.
    pub fn byte_str(&self) -> Option<ByteStrParam> {
        match self.kind {
            FlatParamKind::ByteStr { mode, .. } => Some(mode),
            _ => None,
        }
    }

    /// Whether this parameter travels as an `i64`/`u64` `(lo, hi)` pair.
    pub fn is_i64_pair(&self) -> bool {
        matches!(self.kind, FlatParamKind::I64Pair)
    }

    /// The tagged-product shape when this is a `Tagged` parameter.
    pub fn tagged(&self) -> Option<&TaggedShape> {
        match &self.kind {
            FlatParamKind::Tagged(shape) => Some(shape),
            _ => None,
        }
    }

    /// The recursive fixed-product shape when this is a `Recursive` parameter.
    pub fn recursive(&self) -> Option<&RecursiveShape> {
        match &self.kind {
            FlatParamKind::Recursive(shape) => Some(shape),
            _ => None,
        }
    }

    pub(crate) fn recursive_mut(&mut self) -> Option<&mut RecursiveShape> {
        match &mut self.kind {
            FlatParamKind::Recursive(shape) => Some(shape),
            _ => None,
        }
    }

    /// How many JavaScript parameters the flat parameter takes.
    pub fn fields(&self) -> u32 {
        match &self.kind {
            FlatParamKind::Aggregate { passed, .. } | FlatParamKind::MutEnum { passed, .. } => {
                passed.len() as u32
            }
            FlatParamKind::ScalarRef { .. } => 1,
            FlatParamKind::Slice { zero_offset, .. } => {
                if *zero_offset {
                    2
                } else {
                    3
                }
            }
            FlatParamKind::ByteStr { mode, .. } => match mode {
                ByteStrParam::Full => 1,
                ByteStrParam::Sub => 3,
            },
            FlatParamKind::I64Pair => 2,
            FlatParamKind::Tagged(shape) => shape.fields(),
            FlatParamKind::Recursive(shape) => shape.fields(),
        }
    }

    /// Number of JIR-R parameters this parameter occupies after `rewrite_callee_with`.
    pub(crate) fn jir_param_arity(&self) -> usize {
        match &self.kind {
            FlatParamKind::Aggregate { passed, .. } | FlatParamKind::MutEnum { passed, .. } => {
                passed.len()
            }
            FlatParamKind::ScalarRef { .. }
            | FlatParamKind::Slice { .. }
            | FlatParamKind::ByteStr { .. }
            | FlatParamKind::I64Pair
            | FlatParamKind::Tagged(_)
            | FlatParamKind::Recursive(_) => 1,
        }
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.type_id(self.ty);
        self.kind.hash_into(feed);
    }
}

/// One physical component of a semantic leaf value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaneComponent {
    /// The JavaScript value itself.
    Value,
    /// The backing buffer of a slot or fat slice pointer.
    Buffer,
    /// The element offset of a slot or fat slice pointer.
    Offset,
    /// The element length of a fat slice pointer.
    Length,
}

impl LaneComponent {
    pub(crate) fn hash_into(self, feed: &mut HashFeed) {
        feed.u8(match self {
            Self::Value => 0,
            Self::Buffer => 1,
            Self::Offset => 2,
            Self::Length => 3,
        });
    }
}

/// A recursively physical fixed product.
///
/// `Product` preserves the source aggregate's declaration-order fields, while `Leaf` describes
/// the one-to-three JavaScript lanes that carry an indivisible semantic leaf. Zero-sized fields
/// remain in the tree so canonical reconstruction retains the source shape without spending an
/// ABI lane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductNode {
    Zst {
        ty: TypeId,
    },
    Leaf {
        ty: TypeId,
        components: Vec<LaneComponent>,
    },
    Product {
        ty: TypeId,
        fields: Vec<ProductNode>,
    },
}

impl ProductNode {
    pub(crate) fn fields(&self) -> u32 {
        match self {
            ProductNode::Zst { .. } => 0,
            ProductNode::Leaf { components, .. } => components.len() as u32,
            ProductNode::Product { fields, .. } => fields.iter().map(ProductNode::fields).sum(),
        }
    }

    pub(crate) fn has_off_zero_slice_leaf(&self) -> bool {
        match self {
            ProductNode::Zst { .. } => false,
            ProductNode::Leaf { components, .. } => {
                components.as_slice() == [LaneComponent::Buffer, LaneComponent::Length]
            }
            ProductNode::Product { fields, .. } => fields.iter().any(Self::has_off_zero_slice_leaf),
        }
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        match self {
            ProductNode::Zst { ty } => {
                feed.u8(0);
                feed.type_id(*ty);
            }
            ProductNode::Leaf { ty, components } => {
                feed.u8(1);
                feed.type_id(*ty);
                feed.slice(components, |f, c| c.hash_into(f));
            }
            ProductNode::Product { ty, fields } => {
                feed.u8(2);
                feed.type_id(*ty);
                feed.slice(fields, |f, field| field.hash_into(f));
            }
        }
    }
}

/// A fixed product whose nested fields cross a private direct edge as physical leaves.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecursiveShape {
    pub ty: TypeId,
    pub root: ProductNode,
    /// Top-level semantic fields accepted by the result-use proof.
    pub fields: Vec<u32>,
}

impl RecursiveShape {
    pub fn fields(&self) -> u32 {
        self.root.fields()
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.type_id(self.ty);
        self.root.hash_into(feed);
        feed.slice(&self.fields, |f, &idx| f.u32(idx));
    }
}

/// One payload property of a tagged physical product.
///
/// `variant` and `field` name a representative declaration whose JavaScript key carries the
/// lane. When both variants use the same key and type (as `Result<T, T>` does), they share one
/// lane. Missing properties are `undefined` and are never read on the inactive variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedLane {
    pub variant: u32,
    pub field: u32,
    /// Variants carrying the same canonical property with this exact product shape.
    pub present_variants: Vec<u32>,
    pub node: ProductNode,
}

impl TaggedLane {
    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.u32(self.variant);
        feed.u32(self.field);
        feed.slice(&self.present_variants, |f, &v| f.u32(v));
        self.node.hash_into(feed);
    }
}

/// The common product description for a small tagged value: `tag, payload...`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaggedShape {
    pub ty: TypeId,
    pub lanes: Vec<TaggedLane>,
    /// Every semantic payload projection accepted by the shape, including zero-sized fields and
    /// fields that share another variant's physical property lane.
    pub fields: Vec<(u32, u32)>,
}

impl TaggedShape {
    /// The JavaScript arity: one tag plus the live payload properties.
    pub fn fields(&self) -> u32 {
        1 + self
            .lanes
            .iter()
            .map(|lane| lane.node.fields())
            .sum::<u32>()
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.type_id(self.ty);
        feed.slice(&self.lanes, |f, lane| lane.hash_into(f));
        feed.slice(&self.fields, |f, &(v, idx)| {
            f.u32(v);
            f.u32(idx);
        });
    }
}

/// A two-variant payload-bearing enum returned without allocating a `{ TAG, ... }` wrapper:
/// `Ok(v)` returns `v` directly (`undefined` when the `Ok` payload is ZST) and `Err(e)` writes
/// `e` to spill register 1 (when the `Err` payload is non-ZST) and returns a private module-scope
/// sentinel object (`const $se = {};`).
///
/// Unlike [`TaggedShape`], this allows `needs_drop == true` (returning moves ownership without
/// cloning), recursive / mutually-recursive SCCs, `RTerm::Invoke` call sites, and escaping callers
/// (which materialize canonical `{ TAG, ... }` at the call site).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SentinelResultShape {
    pub ty: TypeId,
    pub ok_variant: u32,
    pub ok_field: Option<(u32, TypeId)>,
    pub err_variant: u32,
    pub err_field: Option<(u32, TypeId)>,
}

impl SentinelResultShape {
    /// Physical result lanes: 2 when `err_field` is non-ZST (primary return + spill register 1),
    /// or 1 when `err_field` is ZST (primary return only — zero spill registers!).
    pub const fn fields(&self) -> u32 {
        if self.err_field.is_some() {
            2
        } else {
            1
        }
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.type_id(self.ty);
        feed.u32(self.ok_variant);
        feed.opt(self.ok_field, |f, (idx, ty)| {
            f.u32(idx);
            f.type_id(ty);
        });
        feed.u32(self.err_variant);
        feed.opt(self.err_field, |f, (idx, ty)| {
            f.u32(idx);
            f.type_id(ty);
        });
    }
}

/// Exclusive physical representation of a flattened return value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlatResultKind {
    /// A fixed-shape aggregate whose non-ZST fields return via primary return + spill registers.
    Aggregate {
        /// The fields in declaration order, which is also the caller's rebuild order.
        field_tys: Vec<TypeId>,
        /// The non-zero-sized fields that travel as result lanes, by index into `field_tys`.
        passed: Vec<u32>,
    },
    /// A 64-bit integer (`i64`/`u64`) returned as `(lo, hi)` (`lo` primary, `hi` in spill 1).
    I64Pair,
    /// A small two-variant enum returned as `(tag, payload...)` with payloads in spill lanes.
    Tagged(TaggedShape),
    /// A nested fixed product that expands only at Phase C JS lowering.
    Recursive(RecursiveShape),
    /// Sentinel flat-return form for two-variant `Result`-shaped enums under `ReleaseSpeed`.
    Sentinel(SentinelResultShape),
}

impl FlatResultKind {
    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        match self {
            Self::Aggregate { field_tys, passed } => {
                feed.u8(1);
                feed.slice(field_tys, |f, &ty| f.type_id(ty));
                feed.slice(passed, |f, &idx| f.u32(idx));
            }
            Self::I64Pair => feed.u8(2),
            Self::Tagged(shape) => {
                feed.u8(3);
                shape.hash_into(feed);
            }
            Self::Recursive(shape) => {
                feed.u8(4);
                shape.hash_into(feed);
            }
            Self::Sentinel(shape) => {
                feed.u8(5);
                shape.hash_into(feed);
            }
        }
    }
}

/// One aggregate/enum/scalar result the plan returns as independent physical lanes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlatResult {
    /// The type the baseline signature returns.
    pub ty: TypeId,
    /// Exclusive physical return representation.
    pub kind: FlatResultKind,
}

impl FlatResult {
    pub fn aggregate(ty: TypeId, field_tys: Vec<TypeId>, passed: Vec<u32>) -> Self {
        Self {
            ty,
            kind: FlatResultKind::Aggregate { field_tys, passed },
        }
    }

    pub fn i64_pair(ty: TypeId) -> Self {
        Self {
            ty,
            kind: FlatResultKind::I64Pair,
        }
    }

    pub fn tagged_result(ty: TypeId, shape: TaggedShape) -> Self {
        Self {
            ty,
            kind: FlatResultKind::Tagged(shape),
        }
    }

    pub fn recursive_result(ty: TypeId, shape: RecursiveShape) -> Self {
        Self {
            ty,
            kind: FlatResultKind::Recursive(shape),
        }
    }

    pub fn sentinel_result(ty: TypeId, shape: SentinelResultShape) -> Self {
        Self {
            ty,
            kind: FlatResultKind::Sentinel(shape),
        }
    }

    pub fn is_aggregate(&self) -> bool {
        matches!(self.kind, FlatResultKind::Aggregate { .. })
    }

    pub fn is_i64_pair(&self) -> bool {
        matches!(self.kind, FlatResultKind::I64Pair)
    }

    pub fn tagged(&self) -> Option<&TaggedShape> {
        match &self.kind {
            FlatResultKind::Tagged(shape) => Some(shape),
            _ => None,
        }
    }

    pub fn recursive(&self) -> Option<&RecursiveShape> {
        match &self.kind {
            FlatResultKind::Recursive(shape) => Some(shape),
            _ => None,
        }
    }

    pub fn sentinel(&self) -> Option<SentinelResultShape> {
        match self.kind {
            FlatResultKind::Sentinel(shape) => Some(shape),
            _ => None,
        }
    }

    /// How many physical result lanes this semantic result takes.
    pub fn fields(&self) -> u32 {
        match &self.kind {
            FlatResultKind::Aggregate { passed, .. } => passed.len() as u32,
            FlatResultKind::I64Pair => 2,
            FlatResultKind::Tagged(shape) => shape.fields(),
            FlatResultKind::Recursive(shape) => shape.fields(),
            FlatResultKind::Sentinel(shape) => shape.fields(),
        }
    }

    /// Whether a value returned with this result shape can be forwarded whole into `param`.
    pub(crate) fn matches_by_value_param(&self, param: &FlatParam) -> bool {
        if self.ty != param.ty {
            return false;
        }
        match (&self.kind, &param.kind) {
            (
                FlatResultKind::Aggregate {
                    field_tys: rf,
                    passed: rp,
                },
                FlatParamKind::Aggregate {
                    field_tys: pf,
                    passed: pp,
                    access: ParamAccess::ByValue,
                },
            ) => rf == pf && rp == pp,
            (FlatResultKind::Tagged(rt), FlatParamKind::Tagged(pt)) => rt == pt,
            (FlatResultKind::Recursive(rr), FlatParamKind::Recursive(pr)) => rr == pr,
            _ => false,
        }
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.type_id(self.ty);
        self.kind.hash_into(feed);
    }
}

/// One callee's flat signature: what happens to each parameter, and how many spill registers its
/// out-fields need.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FlatSig {
    /// One entry per JavaScript parameter of the baseline signature; `None` is unchanged.
    pub params: Vec<Option<FlatParam>>,
    /// The aggregate result returned as lanes, or `None` when the result stays canonical.
    pub result: Option<FlatResult>,
    /// How many spill registers the callee writes. Registers are numbered from 1.
    pub spill: u32,
    /// Whether the callee's `()` return carries the first out-field instead.
    pub returns_out_field: bool,
}

impl FlatSig {
    /// Yields `(jir_position, &FlatParam)` for each flattened parameter after JIR-level parameter
    /// expansion (`Aggregate` / `MutEnum` / `ScalarRef` expand in JIR; physical-only params remain
    /// 1 semantic JIR parameter).
    pub(crate) fn jir_param_positions(&self) -> impl Iterator<Item = (usize, &FlatParam)> {
        let mut cursor = 0usize;
        self.params.iter().filter_map(move |param| match param {
            Some(flat) => {
                let pos = cursor;
                cursor += flat.jir_param_arity();
                Some((pos, flat))
            }
            None => {
                cursor += 1;
                None
            }
        })
    }

    /// Positions of 3-lane `(buf, off, len)` slice values after aggregate parameters have already
    /// expanded in JIR-R. Phase C uses these positions to expand only canonical 3-lane slices;
    /// zero-offset 2-lane slices (`is_off_zero_slice`) expand through
    /// [`Self::off_zero_slice_jir_positions`] in the callee and [`Self::recursive_jir_positions`]
    /// at direct call sites.
    pub(crate) fn slice_jir_positions(&self) -> Vec<usize> {
        self.jir_param_positions()
            .filter_map(|(pos, param)| {
                matches!(
                    param.kind,
                    FlatParamKind::Slice {
                        zero_offset: false,
                        ..
                    }
                )
                .then_some(pos)
            })
            .collect()
    }

    /// Positions of zero-offset 2-lane `(buf, len)` slice values in JIR parameter numbering.
    pub(crate) fn off_zero_slice_jir_positions(&self) -> Vec<usize> {
        self.jir_param_positions()
            .filter_map(|(pos, param)| {
                matches!(
                    param.kind,
                    FlatParamKind::Slice {
                        zero_offset: true,
                        ..
                    }
                )
                .then_some(pos)
            })
            .collect()
    }

    /// Positions of read-only byte-string `&[u8]` parameters in JIR parameter numbering.
    pub(crate) fn byte_str_jir_positions(&self) -> Vec<(usize, ByteStrParam)> {
        self.jir_param_positions()
            .filter_map(|(pos, param)| match param.kind {
                FlatParamKind::ByteStr { mode, .. } => Some((pos, mode)),
                _ => None,
            })
            .collect()
    }

    /// Tagged values keep one semantic JIR parameter, just as slices do, while phase C expands
    /// that position to `tag, payload...` in the JavaScript declaration and direct calls.
    pub(crate) fn tagged_jir_positions(&self) -> Vec<(usize, TaggedShape)> {
        self.jir_param_positions()
            .filter_map(|(pos, param)| match &param.kind {
                FlatParamKind::Tagged(shape) => Some((pos, shape.clone())),
                _ => None,
            })
            .collect()
    }

    /// Recursive products also remain one semantic JIR value and expand only in JavaScript.
    /// Zero-offset 2-lane slices (`is_off_zero_slice`) expose their `[Buffer, Length]` leaf shape
    /// here so direct call lowering emits `(buf, len)` with `reread` evaluation protection.
    pub(crate) fn recursive_jir_positions(&self) -> Vec<(usize, RecursiveShape)> {
        self.jir_param_positions()
            .filter_map(|(pos, param)| match &param.kind {
                FlatParamKind::Recursive(shape) => Some((pos, shape.clone())),
                FlatParamKind::Slice {
                    zero_offset: true, ..
                } => Some((
                    pos,
                    RecursiveShape {
                        ty: param.ty,
                        root: ProductNode::Leaf {
                            ty: param.ty,
                            components: vec![LaneComponent::Buffer, LaneComponent::Length],
                        },
                        fields: Vec::new(),
                    },
                )),
                _ => None,
            })
            .collect()
    }

    /// Positions of 64-bit integer `(lo, hi)` pair parameters in JIR parameter numbering.
    pub(crate) fn i64_pair_jir_positions(&self) -> Vec<usize> {
        self.jir_param_positions()
            .filter_map(|(pos, param)| matches!(param.kind, FlatParamKind::I64Pair).then_some(pos))
            .collect()
    }

    /// The out-fields the callee copies back, as `(parameter position, field index)`, in the order
    /// both ends walk them.
    pub(crate) fn out_fields(&self) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        for (position, param) in self.params.iter().enumerate() {
            let Some(param) = param else { continue };
            match &param.kind {
                FlatParamKind::Aggregate {
                    passed,
                    access: ParamAccess::MutRef { written: true },
                    ..
                }
                | FlatParamKind::MutEnum {
                    passed,
                    written: true,
                    ..
                } => {
                    out.extend(passed.iter().map(|field| (position, *field)));
                }
                FlatParamKind::ScalarRef {
                    mutable: true,
                    written: true,
                } => {
                    out.push((position, 0));
                }
                _ => {}
            }
        }
        out
    }

    /// The type of out-field `out`.
    pub(crate) fn out_ty(&self, out: (usize, u32)) -> TypeId {
        let param = self.params[out.0]
            .as_ref()
            .expect("an out-field names a flat parameter");
        match &param.kind {
            FlatParamKind::Aggregate { field_tys, .. }
            | FlatParamKind::MutEnum { field_tys, .. } => field_tys[out.1 as usize],
            FlatParamKind::ScalarRef { .. } => param.ty,
            _ => unreachable!("non-out parameter in out_fields"),
        }
    }

    /// Result lanes beyond the primary JavaScript return.
    pub(crate) fn result_spills(&self) -> u32 {
        self.result.as_ref().map_or(0, |result| result.fields() - 1)
    }

    pub(crate) fn hash_into(&self, feed: &mut HashFeed) {
        feed.slice(&self.params, |f, param| {
            f.opt(param.as_ref(), |f, p| p.hash_into(f));
        });
        feed.opt(self.result.as_ref(), |f, r| r.hash_into(f));
        feed.u32(self.spill);
        feed.bool(self.returns_out_field);
    }
}

/// Formats a compact human-readable signature shape descriptor for sidecar reports.
pub(crate) fn signature_shape(sig: &FlatSig) -> String {
    let mut parts = Vec::new();
    for param in sig.params.iter().flatten() {
        let kind = match &param.kind {
            FlatParamKind::Slice { .. } => "slice",
            FlatParamKind::ByteStr {
                mode: ByteStrParam::Full,
                ..
            } => "bstr-full",
            FlatParamKind::ByteStr {
                mode: ByteStrParam::Sub,
                ..
            } => "bstr-sub",
            FlatParamKind::I64Pair => "i64-pair",
            FlatParamKind::Tagged(_) => "tagged",
            FlatParamKind::MutEnum { .. } => "mut-enum",
            FlatParamKind::ScalarRef { .. } => "scalar-ref",
            FlatParamKind::Recursive(_) => "recursive-product",
            FlatParamKind::Aggregate { .. } => "product",
        };
        parts.push(format!("{kind}({})", param.fields()));
    }
    if let Some(result) = &sig.result {
        let kind = match &result.kind {
            FlatResultKind::I64Pair => "i64-pair-result",
            FlatResultKind::Sentinel(_) => "sentinel-result",
            FlatResultKind::Tagged(_) => "tagged-result",
            FlatResultKind::Recursive(_) => "recursive-product-result",
            FlatResultKind::Aggregate { .. } => "product-result",
        };
        parts.push(format!("{kind}({})", result.fields()));
    }
    parts.join("+")
}

/// The flat shape of a parameter of type `ty`, or `None` when it has none.
///
/// A candidate aggregate by value, or a reference to one. The field types come from
/// [`crate::opt::candidates::shape`], so the flat ABI and the escape analysis agree about which
/// aggregates have fields worth naming — which is the point: the callee's rebuilt aggregate is
/// only worth building because the escape analysis takes it apart again.
#[cfg(test)]
pub(super) fn flat_param(table: &TypeTable, ty: TypeId, written: bool) -> Option<FlatParam> {
    flat_param_with(None, table, ty, written)
}

pub(super) fn flat_param_with(
    func: Option<&Func>,
    table: &TypeTable,
    ty: TypeId,
    written: bool,
) -> Option<FlatParam> {
    let unpeeled_desc = table.get(ty);
    let (aggregate, by_ref, mutable) =
        match unpeeled_desc.map(|d| (&d.kind, d.meta.boundary, d.js.transparent)) {
            Some((TypeKind::Reference { pointee, mutable }, PinnedBoundary::None, None)) => {
                (*pointee, true, *mutable)
            }
            _ => (ty, false, false),
        };
    if by_ref {
        let is_slot = unpeeled_desc.is_some_and(|d| d.js.pointer_form == Some(PointerForm::Slot));
        if is_slot && is_flattenable_scalar(table, aggregate) {
            return Some(FlatParam::scalar_ref(aggregate, mutable, written));
        }
        if mutable {
            if let Some(mut_enum) = mut_enum_shape(func, table, aggregate, None) {
                let mut field_tys = Vec::with_capacity(1 + mut_enum.payload_tys.len());
                field_tys.push(mut_enum.discr_ty);
                field_tys.extend_from_slice(&mut_enum.payload_tys);
                let passed = (0..field_tys.len() as u32).collect();
                return Some(FlatParam::mut_enum(aggregate, field_tys, passed, written));
            }
        }
    }
    if !by_ref {
        if crate::lower::js::i64_pair::is_i64(table, aggregate) {
            return Some(FlatParam::i64_pair(aggregate));
        }
        if let Some(tagged) = tagged_shape(table, aggregate) {
            return Some(FlatParam::tagged_param(aggregate, tagged));
        }
        if let Some(recursive) = recursive_shape(table, aggregate) {
            return Some(FlatParam::recursive_param(aggregate, recursive));
        }
    }
    let (field_tys, passed) = flat_shape(table, aggregate)?;
    // The rewrite spells the reference as a borrow of the rebuilt aggregate with an empty path,
    // which the verifier allows only for a value with storage to point at.
    if by_ref && !table.is_indirect(aggregate) {
        return None;
    }
    let access = match (by_ref, mutable) {
        (false, _) => ParamAccess::ByValue,
        (true, false) => ParamAccess::SharedRef,
        (true, true) => ParamAccess::MutRef { written },
    };
    Some(FlatParam::aggregate(aggregate, field_tys, passed, access))
}

/// A slice pointer that can travel as its JavaScript buffer, offset, and scalar length.
pub(super) fn slice_param(
    func: &Func,
    table: &TypeTable,
    param: ValueId,
    ty: TypeId,
) -> Option<FlatParam> {
    let pointee = match table
        .get(table.peeled(ty))
        .map(|descriptor| &descriptor.kind)
    {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => return None,
    };
    if !matches!(
        table
            .get(table.peeled(pointee))
            .map(|descriptor| &descriptor.kind),
        Some(TypeKind::Slice(_))
    ) {
        return None;
    }
    let usize_ty = func
        .blocks
        .iter()
        .flat_map(|block| &block.ops)
        .find_map(|op| match op.kind {
            OpKind::Unary {
                op: crate::jir::semantic::UnaryOp::PtrMetadata,
                v,
                ..
            } if v == param => op.result.and_then(|result| func.ty(result)),
            _ => None,
        })
        .or_else(|| {
            table.descriptors().iter().find_map(|descriptor| {
                matches!(
                    descriptor.kind,
                    TypeKind::Integer {
                        signed: false,
                        bits: 32
                    }
                )
                .then_some(descriptor.id)
            })
        })?;
    // The first type is an opaque physical buffer lane. It deliberately retains the slice's
    // stable type identity in JIR reporting; JavaScript lowering never treats it as a Rust
    // pointer value on its own.
    Some(FlatParam::slice(ty, vec![ty, usize_ty, usize_ty]))
}

/// The flat result shape of `ty`, or `None` when it must stay one materialized value.
#[cfg(test)]
pub(super) fn flat_result(table: &TypeTable, ty: TypeId) -> Option<FlatResult> {
    flat_result_with(table, ty, false)
}

/// The flat result shape of `ty`, optionally enabling [`SentinelResultShape`] under
/// `Profile::ReleaseSpeed` when `opts.chunks.is_empty()`.
pub(super) fn flat_result_with(
    table: &TypeTable,
    ty: TypeId,
    allow_sentinel: bool,
) -> Option<FlatResult> {
    if crate::lower::js::i64_pair::is_i64(table, ty) {
        return Some(FlatResult::i64_pair(ty));
    }
    if allow_sentinel {
        if let Some(sentinel) = sentinel_result_shape(table, ty) {
            return Some(FlatResult::sentinel_result(ty, sentinel));
        }
    }
    if let Some(tagged) = tagged_shape(table, ty) {
        return Some(FlatResult::tagged_result(ty, tagged));
    }
    if let Some(recursive) = recursive_shape(table, ty) {
        return Some(FlatResult::recursive_result(ty, recursive));
    }
    let (field_tys, passed) = flat_shape(table, ty)?;
    Some(FlatResult::aggregate(ty, field_tys, passed))
}

/// Checks whether `ty` is an eligible two-variant payload-bearing enum for [`SentinelResultShape`].
///
/// Unlike [`tagged_shape`], this deliberately allows `descriptor.layout.needs_drop == true`
/// because returning `Ok(v)` / `Err(e)` moves ownership without copying or cloning.
pub(crate) fn sentinel_result_shape(table: &TypeTable, ty: TypeId) -> Option<SentinelResultShape> {
    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized
        || descriptor.layout.zst
        || descriptor.meta.abi == NominalAbi::C
        || descriptor.meta.boundary != PinnedBoundary::None
        || descriptor.js.transparent.is_some()
        || descriptor.js.unsized_tail.is_some()
        || !matches!(
            descriptor.js.enum_repr,
            Some(EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false })
        )
    {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        niche,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    if variants.len() != 2 {
        return None;
    }
    if niche.is_none() && variants.iter().all(|v| v.physical_tag.is_none()) {
        return None;
    }
    let single_non_zst = |variant_index: u32| -> Option<Option<(u32, TypeId)>> {
        let variant = variants.get(variant_index as usize)?;
        if variant
            .fields
            .iter()
            .any(|f| crate::lower::js::value::is_uninhabited(table, f.ty))
        {
            return None;
        }
        let mut found = None;
        for (fi, field) in variant.fields.iter().enumerate() {
            table.field(ty, Some(variant_index), fi as u32)?;
            if table.is_zst(field.ty) {
                continue;
            }
            if field.key == FieldKey::Transparent || found.is_some() {
                return None;
            }
            found = Some((fi as u32, field.ty));
        }
        Some(found)
    };
    let f0 = single_non_zst(0)?;
    let f1 = single_non_zst(1)?;
    if f0.is_none() && f1.is_none() {
        return None;
    }
    let (ok_variant, ok_field, err_variant, err_field) = if variants[0].result_ok {
        (0, f0, 1, f1)
    } else if variants[1].result_ok {
        (1, f1, 0, f0)
    } else if f0.is_some() && f1.is_none() {
        (0, f0, 1, None)
    } else if f1.is_some() && f0.is_none() {
        (1, f1, 0, None)
    } else {
        (0, f0, 1, f1)
    };
    Some(SentinelResultShape {
        ty,
        ok_variant,
        ok_field,
        err_variant,
        err_field,
    })
}

/// Whether a leaf tagged consumer returns one scalar JavaScript value.
///
/// `flat_result == None` is not this test: it also means an unsupported aggregate such as
/// `ControlFlow`, which is precisely the tag-to-tag boundary this tranche must leave canonical.
pub(super) fn scalar_leaf_result(table: &TypeTable, ty: TypeId) -> bool {
    matches!(
        table
            .get(table.peeled(ty))
            .map(|descriptor| &descriptor.kind),
        Some(
            TypeKind::Bool
                | TypeKind::Char
                | TypeKind::Integer { .. }
                | TypeKind::Float { .. }
                | TypeKind::Str
                | TypeKind::Never
        )
    ) || table.is_primitive_enum(ty)
}

/// Whether an operation in a tagged leaf stays inside scalar computation.
pub(super) fn scalar_leaf_op(table: &TypeTable, func: &Func, op: &Op) -> bool {
    let scalar_signature = |args: &[ValueId]| {
        args.iter().all(|argument| {
            func.ty(*argument)
                .is_some_and(|ty| scalar_leaf_result(table, ty))
        }) && op.result.is_none_or(|result| {
            func.ty(result)
                .is_some_and(|ty| table.is_zst(ty) || scalar_leaf_result(table, ty))
        })
    };
    match &op.kind {
        OpKind::Call {
            callee: Callee::Direct { .. },
            function: None,
            args,
            host: None,
            ..
        }
        | OpKind::Intrinsic { args, .. } => scalar_signature(args),
        OpKind::Call { .. } | OpKind::Host { .. } | OpKind::Drop { .. } => false,
        _ => true,
    }
}

/// The bounded tagged-product shape shared by `Option`, `Result`, and ordinary two-variant enums.
pub(super) fn tagged_shape(table: &TypeTable, ty: TypeId) -> Option<TaggedShape> {
    const MAX_TAGGED_LANES: u32 = 12;

    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized
        || descriptor.layout.zst
        || descriptor.layout.needs_drop
        || descriptor.meta.abi == NominalAbi::C
        || descriptor.meta.boundary != PinnedBoundary::None
        || descriptor.js.transparent.is_some()
        || descriptor.js.unsized_tail.is_some()
        || !matches!(
            descriptor.js.enum_repr,
            Some(
                EnumRepr::Tag { fieldless: false }
                    | EnumRepr::IntTag { fieldless: false }
                    | EnumRepr::State
            )
        )
    {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    if variants.len() != 2 {
        return None;
    }

    let mut keyed: Vec<(FieldKey, TaggedLane)> = Vec::new();
    let mut fields = Vec::new();
    for (variant, descriptor) in variants.iter().enumerate() {
        for (field, descriptor) in descriptor.fields.iter().enumerate() {
            fields.push((variant as u32, field as u32));
            if table.is_zst(descriptor.ty) {
                continue;
            }
            if descriptor.key == FieldKey::Transparent {
                return None;
            }
            let node = product_node(table, descriptor.ty, 1)?;
            if let Some((_, lane)) = keyed.iter_mut().find(|(key, _)| *key == descriptor.key) {
                if lane.node != node {
                    return None;
                }
                lane.present_variants.push(variant as u32);
                continue;
            }
            keyed.push((
                descriptor.key,
                TaggedLane {
                    variant: variant as u32,
                    field: field as u32,
                    present_variants: vec![variant as u32],
                    node,
                },
            ));
            if 1 + keyed
                .iter()
                .map(|(_, lane)| lane.node.fields())
                .sum::<u32>()
                > MAX_TAGGED_LANES
            {
                return None;
            }
        }
    }
    if keyed.is_empty() {
        return None;
    }
    Some(TaggedShape {
        ty,
        lanes: keyed.into_iter().map(|(_, lane)| lane).collect(),
        fields,
    })
}

const MAX_RECURSIVE_DEPTH: usize = 4;
const MAX_RECURSIVE_FIELDS: usize = 8;
const MAX_RECURSIVE_LANES: u32 = 12;

/// A nested fixed product worth expanding at the JavaScript ABI boundary.
pub(super) fn recursive_shape(table: &TypeTable, ty: TypeId) -> Option<RecursiveShape> {
    let root = product_node(table, ty, 0)?;
    let ProductNode::Product { fields, .. } = &root else {
        return None;
    };
    if root.fields() == 0
        || root.fields() > MAX_RECURSIVE_LANES
        || fields.iter().all(simple_product_field)
    {
        return None;
    }
    let semantic_fields = fields
        .iter()
        .enumerate()
        .filter_map(|(field, node)| (node.fields() > 0).then_some(field as u32))
        .collect();
    Some(RecursiveShape {
        ty,
        root,
        fields: semantic_fields,
    })
}

fn simple_product_field(node: &ProductNode) -> bool {
    match node {
        ProductNode::Zst { .. } => true,
        ProductNode::Leaf { components, .. } => components.as_slice() == [LaneComponent::Value],
        ProductNode::Product { .. } => false,
    }
}

/// The physical tree of one copyable semantic value.
pub(super) fn product_node(table: &TypeTable, ty: TypeId, depth: usize) -> Option<ProductNode> {
    if depth > MAX_RECURSIVE_DEPTH {
        return None;
    }
    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized || descriptor.layout.needs_drop {
        return None;
    }
    if descriptor.layout.zst {
        return Some(ProductNode::Zst { ty });
    }

    if let Some(pointer) = descriptor.js.pointer_form {
        let components = match pointer {
            PointerForm::Thin | PointerForm::Str => vec![LaneComponent::Value],
            PointerForm::Slot => {
                // Only Rust references (`&T` / `&mut T`) to non-ZST pointees are guaranteed to be
                // plain `{ buf, off }` slot records. Raw pointers and transparent wrappers may
                // carry bare integer addresses or scaled byte-view records (`sc`/`w`/`es`).
                let is_non_zst_ref = matches!(
                    table.get(table.peeled(ty)).map(|d| &d.kind),
                    Some(TypeKind::Reference { pointee, .. }) if !table.is_zst(*pointee)
                );
                if is_non_zst_ref {
                    vec![LaneComponent::Buffer, LaneComponent::Offset]
                } else {
                    vec![LaneComponent::Value]
                }
            }
            PointerForm::FatSlice => vec![
                LaneComponent::Buffer,
                LaneComponent::Offset,
                LaneComponent::Length,
            ],
            PointerForm::FatDyn | PointerForm::FatTail => return None,
        };
        return Some(ProductNode::Leaf { ty, components });
    }

    if matches!(
        descriptor.kind,
        TypeKind::Bool
            | TypeKind::Char
            | TypeKind::Integer { .. }
            | TypeKind::Float { .. }
            | TypeKind::Str
            | TypeKind::FnPtr { .. }
    ) || descriptor.js.is_primitive_enum
    {
        return Some(ProductNode::Leaf {
            ty,
            components: vec![LaneComponent::Value],
        });
    }

    if descriptor.meta.abi == NominalAbi::C
        || descriptor.meta.boundary != PinnedBoundary::None
        || descriptor.js.transparent.is_some()
        || descriptor.js.unsized_tail.is_some()
    {
        return None;
    }
    if !matches!(
        descriptor.kind,
        TypeKind::Tuple(_)
            | TypeKind::Array { .. }
            | TypeKind::Closure { .. }
            | TypeKind::Adt {
                form: NominalForm::Struct,
                ..
            }
    ) {
        return None;
    }
    let field_tys = candidates::variant_shape(table, ty, 0)?;
    if field_tys.len() > MAX_RECURSIVE_FIELDS {
        return None;
    }
    let fields = field_tys
        .into_iter()
        .map(|field| product_node(table, field, depth + 1))
        .collect::<Option<Vec<_>>>()?;
    let node = ProductNode::Product { ty, fields };
    (node.fields() <= MAX_RECURSIVE_LANES).then_some(node)
}

/// Declaration-order fields and the non-zero-sized lanes of a fixed aggregate shape.
fn flat_shape(table: &TypeTable, aggregate: TypeId) -> Option<(Vec<TypeId>, Vec<u32>)> {
    if !matches!(
        table.get(aggregate).map(|d| &d.kind),
        Some(TypeKind::Tuple(_) | TypeKind::Array { .. })
            | Some(TypeKind::Adt {
                form: NominalForm::Struct,
                ..
            })
    ) {
        return None;
    }
    let field_tys = candidates::shape(table, aggregate, 0)?;
    if !candidates::field_reads_resolve(table, aggregate, 0, field_tys.len()) {
        return None;
    }
    if !candidates::shape_survives_copy(table, &field_tys) {
        return None;
    }
    let passed: Vec<u32> = (0..field_tys.len() as u32)
        .filter(|field| !table.is_zst(field_tys[*field as usize]))
        .collect();
    if passed.is_empty() {
        return None;
    }
    Some((field_tys, passed))
}

/// The reference type of `(pointee, mutable)` this program has, if any.
fn reference_ty(table: &TypeTable, pointee: TypeId, mutable: bool) -> Option<TypeId> {
    let of = |want: bool| {
        table
            .descriptors()
            .iter()
            .find(|descriptor| {
                matches!(&descriptor.kind, TypeKind::Reference { pointee: p, mutable: m }
                    if *p == pointee && *m == want)
            })
            .map(|descriptor| descriptor.id)
    };
    of(mutable).or_else(|| of(!mutable))
}

/// The type to give the view a copy-back stores through.
pub(super) fn store_view_ty(table: &TypeTable, field: TypeId) -> Option<TypeId> {
    reference_ty(table, field, true).or_else(|| {
        match table.get(table.peeled(field)).map(|d| &d.kind) {
            Some(TypeKind::Reference { .. } | TypeKind::RawPointer { .. }) => None,
            _ => Some(field),
        }
    })
}

/// Whether `types` spells the same fields the plan chose, and has a type for every view the
/// copy-back needs.
pub(super) fn shapes_agree(types: &TypeTable, shapes: &[Option<FlatParam>]) -> bool {
    shapes.iter().flatten().all(|shape| match &shape.kind {
        FlatParamKind::I64Pair => crate::lower::js::i64_pair::is_i64(types, shape.ty),
        FlatParamKind::Slice { field_tys, .. } => {
            types.get(shape.ty).is_some()
                && field_tys[1..].iter().all(|ty| types.get(*ty).is_some())
        }
        FlatParamKind::ByteStr { usize_ty, .. } => {
            types.get(shape.ty).is_some() && types.get(*usize_ty).is_some()
        }
        FlatParamKind::ScalarRef { .. } => is_flattenable_scalar(types, shape.ty),
        FlatParamKind::MutEnum { field_tys, .. } => {
            mut_enum_shape(None, types, shape.ty, Some(field_tys)).is_some()
        }
        FlatParamKind::Tagged(tagged) => tagged_shape(types, shape.ty).as_ref() == Some(tagged),
        FlatParamKind::Recursive(recursive) => {
            recursive_shape(types, shape.ty).as_ref() == Some(recursive)
        }
        FlatParamKind::Aggregate {
            field_tys,
            passed,
            access,
        } => {
            let needs_copy_back = matches!(access, ParamAccess::MutRef { written: true });
            candidates::shape(types, shape.ty, 0).as_deref() == Some(field_tys.as_slice())
                && candidates::field_reads_resolve(types, shape.ty, 0, field_tys.len())
                && (!needs_copy_back
                    || passed
                        .iter()
                        .all(|field| store_view_ty(types, field_tys[*field as usize]).is_some()))
        }
    })
}

/// Whether `ty` is a non-ZST `Copy` 1-lane scalar eligible for `&Scalar` / `&mut Scalar`
/// flattening (`Bool`, `Char`, `Integer { bits <= 32 }`, `Float`, `is_primitive_enum`).
pub(super) fn is_flattenable_scalar(table: &TypeTable, ty: TypeId) -> bool {
    let Some(outer) = table.get(ty) else {
        return false;
    };
    if !outer.layout.sized
        || outer.layout.zst
        || outer.layout.needs_drop
        || outer.js.crosses_transparent_union
        || outer.meta.boundary != PinnedBoundary::None
        || outer.meta.abi == NominalAbi::C
    {
        return false;
    }
    let Some(peeled) = table.get(table.peeled(ty)) else {
        return false;
    };
    if !peeled.layout.sized
        || peeled.layout.zst
        || peeled.layout.needs_drop
        || peeled.js.is_indirect
        || peeled.js.needs_clone
        || peeled.js.pointer_form.is_some()
    {
        return false;
    }
    matches!(
        peeled.kind,
        TypeKind::Bool | TypeKind::Char | TypeKind::Float { .. }
    ) || matches!(peeled.kind, TypeKind::Integer { bits, .. } if bits <= 32)
        || peeled.js.is_primitive_enum
}

fn has_pattern_type(table: &TypeTable, ty: TypeId) -> bool {
    let mut cur = ty;
    for _ in 0..16 {
        let Some(desc) = table.get(cur) else {
            return true;
        };
        if matches!(desc.kind, TypeKind::Pattern(_)) {
            return true;
        }
        if let Some(inner) = desc.js.transparent {
            cur = inner.ty;
            continue;
        }
        return false;
    }
    true
}

#[derive(Clone, Debug)]
pub(super) struct MutEnumVariant {
    pub(super) logical_discr: i128,
    pub(super) field_tys: Vec<TypeId>,
}

#[derive(Clone, Debug)]
pub(super) struct MutEnumShape {
    pub(super) ty: TypeId,
    pub(super) discr_ty: TypeId,
    pub(super) discr_bytes: u8,
    pub(super) variants: Vec<MutEnumVariant>,
    pub(super) payload_tys: Vec<TypeId>,
}

pub(super) fn mut_enum_shape(
    func: Option<&Func>,
    table: &TypeTable,
    ty: TypeId,
    expected_field_tys: Option<&[TypeId]>,
) -> Option<MutEnumShape> {
    const MAX_MUT_ENUM_VARIANTS: usize = 4;
    const MAX_MUT_ENUM_PAYLOADS: usize = 8;

    let descriptor = table.get(ty)?;
    if !descriptor.layout.sized
        || descriptor.layout.zst
        || descriptor.layout.needs_drop
        || descriptor.meta.abi == NominalAbi::C
        || descriptor.meta.boundary != PinnedBoundary::None
        || descriptor.js.transparent.is_some()
        || descriptor.js.unsized_tail.is_some()
        || !table.is_indirect(ty)
        || !matches!(
            descriptor.js.enum_repr,
            Some(EnumRepr::Tag { fieldless: false } | EnumRepr::IntTag { fieldless: false })
        )
    {
        return None;
    }
    let TypeKind::Adt {
        form: NominalForm::Enum,
        variants,
        ..
    } = &descriptor.kind
    else {
        return None;
    };
    if !(2..=MAX_MUT_ENUM_VARIANTS).contains(&variants.len()) {
        return None;
    }

    let mut payload_tys = Vec::new();
    let mut out_variants = Vec::with_capacity(variants.len());
    for (variant_index, variant) in variants.iter().enumerate() {
        if !candidates::field_reads_resolve(table, ty, variant_index as u32, variant.fields.len()) {
            return None;
        }
        let mut field_tys = Vec::with_capacity(variant.fields.len());
        for field in &variant.fields {
            field_tys.push(field.ty);
            if table.is_zst(field.ty) {
                continue;
            }
            if !is_flattenable_scalar(table, field.ty)
                || has_pattern_type(table, field.ty)
                || default_const_tree(table, field.ty).is_none()
            {
                return None;
            }
            payload_tys.push(field.ty);
            if payload_tys.len() > MAX_MUT_ENUM_PAYLOADS {
                return None;
            }
        }
        out_variants.push(MutEnumVariant {
            logical_discr: variant.logical_discr,
            field_tys,
        });
    }
    if payload_tys.is_empty() {
        return None;
    }

    let (discr_ty, discr_bytes) = if let Some(expected) = expected_field_tys {
        let (&discr_ty, rest) = expected.split_first()?;
        if rest != payload_tys.as_slice() {
            return None;
        }
        let bytes = fits_discr_ty(table, discr_ty, &out_variants)?;
        (discr_ty, bytes)
    } else {
        enum_discr_type(func, table, ty, &out_variants)?
    };

    Some(MutEnumShape {
        ty,
        discr_ty,
        discr_bytes,
        variants: out_variants,
        payload_tys,
    })
}

fn fits_int(value: i128, signed: bool, bits: u32) -> bool {
    if bits == 0 {
        return false;
    }
    if signed {
        bits >= 128 || {
            let bound = 1i128 << (bits - 1);
            value >= -bound && value < bound
        }
    } else {
        value >= 0 && (bits >= 128 || (value as u128) < (1u128 << bits))
    }
}

fn fits_discr_ty(table: &TypeTable, ty: TypeId, variants: &[MutEnumVariant]) -> Option<u8> {
    let (signed, bits) = table.int_info(ty)?;
    if bits > 32 {
        return None;
    }
    let bytes = u8::try_from((bits / 8).max(1)).ok()?;
    variants
        .iter()
        .all(|v| fits_int(v.logical_discr, signed, bits))
        .then_some(bytes)
}

fn enum_discr_type(
    func: Option<&Func>,
    table: &TypeTable,
    enum_ty: TypeId,
    variants: &[MutEnumVariant],
) -> Option<(TypeId, u8)> {
    if let Some(func) = func {
        for block in &func.blocks {
            for op in &block.ops {
                if let OpKind::Discriminant(v) = op.kind {
                    if func
                        .values
                        .get(v.0 as usize)
                        .is_some_and(|d| d.ty == enum_ty)
                    {
                        if let Some(res) = op.result {
                            let res_ty = func.values[res.0 as usize].ty;
                            if let Some(bytes) = fits_discr_ty(table, res_ty, variants) {
                                return Some((res_ty, bytes));
                            }
                        }
                    }
                }
            }
        }
    }
    if let Some((ty, bytes)) = table
        .descriptors()
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                TypeKind::Integer {
                    signed: true,
                    bits: 32
                }
            )
        })
        .find_map(|d| fits_discr_ty(table, d.id, variants).map(|bytes| (d.id, bytes)))
    {
        return Some((ty, bytes));
    }
    table
        .descriptors()
        .iter()
        .find_map(|d| fits_discr_ty(table, d.id, variants).map(|bytes| (d.id, bytes)))
}

pub(super) fn default_const_tree(table: &TypeTable, ty: TypeId) -> Option<ConstTree> {
    let descriptor = table.get(ty)?;
    if descriptor.layout.zst {
        return Some(ConstTree::Unit);
    }
    if let Some(inner) = descriptor.js.transparent {
        return default_const_tree(table, inner.ty);
    }
    match &descriptor.kind {
        TypeKind::Bool => Some(ConstTree::Scalar { bits: 0, bytes: 1 }),
        TypeKind::Char => Some(ConstTree::Scalar { bits: 0, bytes: 4 }),
        TypeKind::Integer { bits, .. } | TypeKind::Float { bits } if *bits <= 64 => {
            Some(ConstTree::Scalar {
                bits: 0,
                bytes: (*bits / 8).max(1) as u8,
            })
        }
        TypeKind::Adt {
            form: NominalForm::Enum,
            variants,
            ..
        } if descriptor.js.is_primitive_enum && !variants.is_empty() => {
            let fields = variants
                .first()?
                .fields
                .iter()
                .map(|field| table.is_zst(field.ty).then_some(ConstTree::Unit))
                .collect::<Option<Vec<_>>>()?;
            Some(ConstTree::Variant { variant: 0, fields })
        }
        _ => None,
    }
}

pub(super) fn mask_bits(bits: u128, bytes: u8) -> u128 {
    match bytes {
        0 => 0,
        1..=15 => bits & ((1_u128 << (u32::from(bytes) * 8)) - 1),
        _ => bits,
    }
}
