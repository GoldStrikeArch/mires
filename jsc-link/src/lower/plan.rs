//! The contract between the SSA side of the lowerer (`ssa.rs`, `fuse.rs`, `structure.rs`) and
//! the JavaScript side (`js/*`): what a lowered item's plan says, and the few helpers both sides
//! spell the same way.
//!
//! The JavaScript side receives three things per item: the JIR-R [`Func`] (`ssa::build`), a
//! [`Plan`] (`fuse::plan`) that says which values are spelled inline, what everything else is
//! called and how block arguments become assignments, and a [`Structured`] region tree
//! (`structure::structure`) whose block payloads point back into the function's ops. This file is
//! frozen once the JavaScript side codes against it; anything added later is added, never
//! changed.
//!
//! # Emission conventions
//!
//! * **Names.** [`Plan::names`] names every entry parameter, every block parameter, and every op
//!   result that is used and not inline; those (parameters aside) are [`Plan::declared_order`], the
//!   `let` prelude. An inline value has no name (its expression is spelled at its use site), and
//!   neither has an op result nothing uses: the emitter runs such an op for its effects only (a
//!   call) or drops it (anything pure). Cells are named in [`Plan::cell_names`]; a boxed cell is
//!   `let x = []` and an unboxed one a plain `let x`. Every name is unique across values, cells and
//!   copy temporaries, never starts with `$s` (the structurizer's `$s{n}` temporaries), never
//!   spells `$loc` for anything but the caller location, and is a plain JavaScript identifier. Rust
//!   names come from `Origin::local_name` (v1 `names.rs` rules); a compiler temporary is `_N` after
//!   its MIR local, an intermediate with no local `$v{n}`, and a copy temporary `$c{n}`.
//! * **Inline values** ([`Plan::inline`]) are defined by an op that produces a value, used exactly
//!   once, in the same block, at an op (or the terminator, or an edge copy) that follows the
//!   definition with only commuting ops in between (v1 `queue.rs`'s purity ladder). The emitter
//!   spells the defining op's expression at the use site and emits nothing at the definition. An
//!   inline value's expression must be spelled once; duplicating it is not the emitter's call to
//!   make.
//! * **Edge copies** ([`Plan::copies`]) are the out-of-SSA form of block arguments: for the edge
//!   `(from, to)` the listed assignments run in order, `dst = src`, after every op of `from` and
//!   before control transfers. They are already sequentialized (a cycle is broken through a
//!   temporary named in [`Plan::copy_temps`]), so a straight run of assignments is correct. The
//!   structurizer payload says where they go: a [`BlockPayload`] with `edge: Some((a, b))` runs
//!   `copies[&(a, b)]` after its `ops`. A `Goto` block carries its own edge; a `Switch` edge with
//!   copies gets a synthesized payload of its own (`block: None`, no ops) on that edge.
//! * **Entry parameters** are `Func.params` in order; the trailing one is the caller `Location`
//!   when `sig.abi.caller_location` (named [`CALLER_LOCATION_NAME`]). A spread tuple arrives as its
//!   `arity` parameters and the SSA form rebuilds the tuple as an `Aggregate` in the entry block. A
//!   parameter whose MIR local is a cell shares its name with the cell ([`Plan::param_cells`]): the
//!   `CellAlloc` + `CellStore` pair at the top of the entry block is the prelude rebinding, `x =
//!   [x]` for a boxed cell and nothing at all for an unboxed one.
//! * **Synthetic views.** A MIR read or write of a projected place that needs an element or pointer
//!   step is a `BorrowView` consumed by exactly one `Deref`/`StoreThrough` right after it; such a
//!   view is always inline. Its type is a real reference type to the place when the type table has
//!   one, the place's own type when that is not a pointer type, and a stand-in non-pointer type of
//!   the table otherwise: the emitter must spell `Deref`/`StoreThrough` of an inline view as v1's
//!   read/write of the walked place, taking the pointee type from the `Deref` result or the stored
//!   value, and never build a pointer record from the view's type. [`view_pointee`] answers for a
//!   value whose type really is a pointer type.
//! * **Discriminants.** A `Discriminant` value listed in [`Plan::tag_values`] reads a tagged enum's
//!   tag (`v.TAG`) and its only uses are switch scrutinees; every other `Discriminant` is the
//!   number (v1 `tag.rs`). [`RScrut::tag_enum`] carries the enum for such a switch, so case values
//!   are spelled as tag literals.
//! * **Switch cases** in the region tree are sign-biased (v1 `cfg_adapter.rs`): spell a case or
//!   test bound through [`decode`]/[`case_value`] with the scrutinee's type.
//! * **Consumed marker arguments.** An argument v1 read at lowering time rather than lowered
//!   (`abi.rs::consumed_arguments`: the `&root` of `hole`/`handler`/`effect`/`component`, the
//!   `&list` of `push`/`pushkeyed`, the `&signal` of `signal`, the `&STATIC` of `template`) is
//!   passed to the marker call as the borrowed value itself when it borrows a local — the value the
//!   `template` call defined — never as a `BorrowView` and never through a box; a `&STATIC` is the
//!   `Const(Global)` it always was. The host lowering identifies the instantiation by that value
//!   and must not spell such an argument as an ordinary operand.
//! * **Store-back idiom.** `discriminant(*p) = k` on an object-form enum lowers to `v = Deref(p);
//!   SetDiscriminant{v}` and nothing else: under the baseline carrier the `Deref` of a pointer to
//!   an indirect pointee is the object itself, so the write lands on the pointee. A primitive
//!   enum's discriminant is the whole value and is written back (`StoreThrough` / `CellStore`); the
//!   same on a cell is `CellLoad; SetDiscriminant; CellStore` (an `x = x`).

use std::collections::{BTreeMap, BTreeSet};

use crate::jir::origin::OriginId;
use crate::jir::repr::{CellId, RBlockId, ValueId};
use crate::jir::semantic::TypeId;
use crate::jir::types::{TypeKind, TypeTable};

/// The origin an op or value carries when the item was encoded without an origin table.
/// `OriginTable::origin` answers `None` for it.
pub const NO_ORIGIN: OriginId = OriginId(u32::MAX);

/// The JavaScript name of the trailing caller `Location` parameter (v1
/// `intrinsics::CALLER_LOCATION_PARAM`).
pub const CALLER_LOCATION_NAME: &str = "$loc";

/// An op inside a JIR-R block, by position: `func.blocks[block].ops[index]`.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpRef {
    pub block: RBlockId,
    pub index: u32,
}

/// One side of an edge copy: a value's name, or a temporary the sequentialization introduced
/// (an index into [`Plan::copy_temps`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CopyOperand {
    Value(ValueId),
    Temp(u32),
}

/// One assignment of an edge's copy sequence: `dst = src`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EdgeCopy {
    pub dst: CopyOperand,
    pub src: CopyOperand,
}

/// The statement payload of one structurizer block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlockPayload {
    /// The JIR-R block whose ops run here; `None` for a payload synthesized on a switch edge,
    /// which carries copies only.
    pub block: Option<RBlockId>,
    /// The ops, in order. Every op of `block` (inline definitions included: the emitter skips
    /// those).
    pub ops: Vec<OpRef>,
    /// The edge whose copies ([`Plan::copies`]) run after `ops`, before the block's jump.
    pub edge: Option<(RBlockId, RBlockId)>,
}

impl structurizer::Payload for BlockPayload {
    /// One unit per op, one for a copy run: how much duplicating this block would duplicate.
    fn cost(&self) -> i32 {
        self.ops.len() as i32 + i32::from(self.edge.is_some())
    }
}

/// The value a branching block tests, with what decides how the tests read it (v1
/// `cfg_adapter::Scrutinee`/`ScrutTy`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RScrut {
    Value {
        value: ValueId,
        ty: TypeId,
        tag_enum: Option<TypeId>,
    },
    /// Whether the protected operation of this block returned normally.
    Invoke(RBlockId),
}

/// What `fuse::plan` decided for one function.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Plan {
    /// Host globals whose lexical bindings the emitted body reads.
    pub globals: BTreeSet<String>,
    /// Values whose expression is spelled at their single use site and never declared.
    pub inline: BTreeSet<ValueId>,
    /// Collision-safe control names for protected operations and their exception.
    pub invoke_names: BTreeMap<RBlockId, String>,
    pub exception_name: Option<String>,
    pub catch_name: Option<String>,
    /// The JavaScript name of every value.
    pub names: BTreeMap<ValueId, String>,
    /// Fat-slice values carried as local `buf`/`off`/`len` lanes. Uses that need the canonical
    /// Rust value read an object literal over those lanes, so materialization happens only at the
    /// actual escape rather than on every loop back edge.
    pub physical_slices: BTreeSet<ValueId>,
    /// Thin pointers derived from physical slices and carried as local `buf`/`off` lanes. This
    /// keeps the compiler-internal pointer record out of a hot loop after a slice-to-element cast.
    pub physical_slots: BTreeSet<ValueId>,
    /// The JavaScript name of every cell.
    pub cell_names: BTreeMap<CellId, String>,
    /// The temporaries the edge copies use, by index; all need a `let`.
    pub copy_temps: Vec<String>,
    /// The out-of-SSA assignments per edge, already sequentialized.
    pub copies: BTreeMap<(RBlockId, RBlockId), Vec<EdgeCopy>>,
    /// The values that need a `let`: not inline, not entry parameters. Deterministic order.
    pub declared_order: Vec<ValueId>,
    /// The scrutinee of every `Switch` block, by block.
    pub scrutinees: BTreeMap<RBlockId, RScrut>,
    /// `Discriminant` values that read a tagged enum's tag rather than its number.
    pub tag_values: BTreeSet<ValueId>,
    /// Cells that hold an entry parameter, with the parameter value: the cell and the parameter
    /// share a name and the entry `CellAlloc`+`CellStore` is the prelude rebinding.
    pub param_cells: BTreeMap<CellId, ValueId>,
    /// The `CellAlloc` ops, by block and index, that create their cell's container: the cell holds
    /// an aggregate that is written in place before anything stores it whole (`lower/assembly.rs`),
    /// so the alloc is spelled `x = []` or `x = {}` rather than nothing.
    pub fresh_allocs: BTreeSet<(RBlockId, u32)>,
    /// JavaScript name to Rust name, for every name whose spelling had to change (the source
    /// map's `names` table; v1 `LocalNames::originals`).
    pub source_names: Vec<(String, String)>,
}

/// The structurizer's output for one function.
#[derive(Debug)]
#[allow(dead_code)] // `stats` is report-only until the lane report reads it.
pub struct Structured {
    pub region: structurizer::Region<BlockPayload, RScrut>,
    pub stats: structurizer::StructureStats,
}

/// The bias that maps the values of `ty` onto the unsigned order: the sign bit, or nothing at
/// all for a type that is already unsigned (v1 `cfg_adapter::bias`).
fn bias(table: &TypeTable, ty: TypeId) -> u128 {
    match table.int_info(ty) {
        Some((true, bits)) if bits > 0 && bits <= 128 => 1u128 << (bits - 1),
        _ => 0,
    }
}

/// A MIR case value as the structurizer should order it: the sign bit flipped for a signed type,
/// so two's complement order becomes unsigned order.
pub fn encode(table: &TypeTable, ty: TypeId, raw: u128) -> u128 {
    raw ^ bias(table, ty)
}

/// The MIR bit pattern an encoded case value stands for.
pub fn decode(table: &TypeTable, ty: TypeId, encoded: u128) -> u128 {
    encoded ^ bias(table, ty)
}

/// Re-signs raw bits through `(signed, bits)` (v1 `value::resign`).
pub fn resign(signed: bool, bits: u32, raw: u128) -> i128 {
    if bits == 0 || bits >= 128 {
        return raw as i128;
    }
    let mask = (1u128 << bits) - 1;
    let raw = raw & mask;
    if signed && raw & (1u128 << (bits - 1)) != 0 {
        (raw | !mask) as i128
    } else {
        raw as i128
    }
}

/// The Rust value an encoded case value stands for, sign included.
pub fn case_value(table: &TypeTable, ty: TypeId, encoded: u128) -> i128 {
    let raw = decode(table, ty, encoded);
    match table.int_info(ty) {
        Some((signed, bits)) => resign(signed, bits, raw),
        None => raw as i128,
    }
}

/// The pointee of a `BorrowView`/`RawView` value of type `view_ty`.
///
/// A real reference or raw pointer type answers its pointee (after peeling transparent
/// wrappers); anything else is a synthetic view, answered with its own type. `ssa.rs` only ever
/// gives a synthetic view a non-pointer type, so the two cases never collide; a synthetic view is
/// consumed by the op after it, which is where its pointee is read from.
pub fn view_pointee(table: &TypeTable, view_ty: TypeId) -> TypeId {
    let peeled = table.peeled(view_ty);
    match table.get(peeled).map(|descriptor| &descriptor.kind) {
        Some(TypeKind::Reference { pointee, .. } | TypeKind::RawPointer { pointee, .. }) => {
            *pointee
        }
        _ => view_ty,
    }
}

/// Whether `ty` may type a synthetic view: its peeled kind is not a pointer, so
/// [`view_pointee`] and the verifier's `Deref` check both read it as the pointee itself.
pub fn placeholder_view_ok(table: &TypeTable, ty: TypeId) -> bool {
    view_pointee(table, ty) == ty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resign_reads_the_sign_bit() {
        assert_eq!(resign(true, 8, 0xff), -1);
        assert_eq!(resign(false, 8, 0xff), 255);
        assert_eq!(resign(true, 32, 5), 5);
        assert_eq!(resign(true, 128, u128::MAX), -1);
    }
}
