//! The MIR body as a control flow graph the structurizer can read.
//!
//! [`build`] lowers every non-cleanup block once — its statements and the side effects of its
//! terminator become the block's payload, the edge the terminator takes becomes its [`Term`] — and
//! hands the result to `structurizer`. `emit.rs` turns the region tree that comes back into
//! JavaScript.
//!
//! # Block numbering
//!
//! Cleanup blocks are left out (nothing reaches them: the backend does not unwind), so MIR's block
//! numbers are not dense and cannot be used as [`BlockId`]s. [`Blocks`] renumbers the ones that
//! survive and maps every edge through, which is also the one place that notices an edge into a
//! block that was dropped — impossible, since only unwind edges enter cleanup blocks, and reported
//! as a throw rather than a panic if it ever happens.
//!
//! # Case values are biased, not raw
//!
//! MIR stores a `SwitchInt` case value as the unsigned bit pattern of the scrutinee's type, so
//! `-1i32` arrives as `4294967295`. The structurizer reads case values as plain integers: it sorts
//! them, groups the contiguous ones and builds range tests out of the result. Handing it raw
//! patterns would make `-1` the *largest* `i32` — `match x { -1 => a, 0 => b }` would look like two
//! far apart singletons, and a `Test::Lt` built from that ordering would be spelled with a `<` that
//! JavaScript evaluates in the signed domain and get the answer wrong.
//!
//! So a signed value is *biased* on the way in — its sign bit is flipped, which maps two's
//! complement order onto unsigned order — and unbiased on the way out ([`decode`], used by every
//! literal `emit.rs` prints). Ordering, contiguity and every range test are then exactly the
//! ordering, contiguity and ranges of the Rust values, and the literals still print signed.

use rustc_middle::mir::START_BLOCK;
use rustc_middle::mir::BasicBlock;
use rustc_middle::ty::{Ty, TyCtxt};
use structurizer::{Block, BlockId, Cfg, Payload};

use crate::base::FnCx;
use crate::jsast::{Expr, Stmt};
use crate::value;

/// The value a branching terminator tests, with the type that decides how the tests read it.
///
/// The type travels with the value because the structurizer only reports the *shape* of a test —
/// "equal to 3", "one of 4 values from 0" — and leaves the spelling to the backend, which needs
/// the signedness and the representation to spell a literal (`3` or `3n` or `-1`).
#[derive(Clone, Copy)]
pub(crate) struct ScrutTy<'tcx> {
    /// The Rust type of the value, which for a discriminant is the integer MIR gave it.
    pub(crate) ty: Ty<'tcx>,
    /// The enum whose **tag** the value is, when it is one (`tag.rs`).
    ///
    /// A tag is a variant's name rather than a number, so the case values the structurizer reports
    /// are spelled as names and the ordering tests it may produce become chains of equalities.
    pub(crate) tag_enum: Option<Ty<'tcx>>,
}

/// The value a branching terminator tests.
pub(crate) struct Scrutinee<'tcx> {
    pub(crate) value: Expr,
    pub(crate) ty: ScrutTy<'tcx>,
}

/// The graph of one function body.
pub(crate) type BodyCfg<'tcx> = Cfg<Vec<Stmt>, Scrutinee<'tcx>>;

/// One unit per statement: a block's cost is how many statements duplicating it would duplicate.
///
/// Block payloads are straight line at this point — the only nested statement a lowering produces
/// here is the `if` guarding an `Assert`'s panic, which is two or three statements' worth — so
/// counting the top level is close enough for the one decision this feeds, whether a loop
/// continuation is small enough to be duplicated into the loop.
impl Payload for Stmt {
    fn cost(&self) -> i32 {
        1
    }
}

/// The non-cleanup blocks of a body, renumbered densely.
pub(crate) struct Blocks {
    /// The new id of each MIR block, `None` for the ones left out.
    ids: Vec<Option<BlockId>>,
}

impl Blocks {
    /// The id a MIR block was given, or `None` if it is a cleanup block.
    pub(crate) fn id(&self, bb: BasicBlock) -> Option<BlockId> {
        self.ids.get(bb.as_usize()).copied().flatten()
    }
}

/// Lowers a body into the graph the structurizer takes.
///
/// Returns `None` only for a body with no reachable entry block, which rustc does not produce.
pub(crate) fn build<'tcx>(fx: &FnCx<'_, 'tcx>) -> Option<BodyCfg<'tcx>> {
    let mir = fx.mir;
    let mut ids = vec![None; mir.basic_blocks.len()];
    let mut order = Vec::with_capacity(mir.basic_blocks.len());
    for (bb, data) in mir.basic_blocks.iter_enumerated() {
        if data.is_cleanup {
            continue;
        }
        ids[bb.as_usize()] = Some(BlockId(order.len() as u32));
        order.push(bb);
    }

    let blocks = Blocks { ids };
    let entry = blocks.id(START_BLOCK)?;

    let mut out = Vec::with_capacity(order.len());
    for bb in order {
        let (stmts, term) = fx.codegen_block(bb, &mir.basic_blocks[bb], &blocks);
        // Unwind edges are not edges of this graph; they are what the try/catch pass will read.
        out.push(Block { stmts, term, unwind: None });
    }
    Some(Cfg { blocks: out, entry })
}

/// The bias that maps the values of `ty` onto the unsigned order: the sign bit, or nothing at all
/// for a type that is already unsigned.
fn bias<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>) -> u128 {
    match value::int_info(tcx, ty) {
        Some((true, bits)) => 1u128 << (bits - 1),
        _ => 0,
    }
}

/// A MIR case value as the structurizer should order it. See the module documentation.
pub(crate) fn encode<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, raw: u128) -> u128 {
    raw ^ bias(tcx, ty)
}

/// The MIR bit pattern an encoded case value stands for.
pub(crate) fn decode<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, encoded: u128) -> u128 {
    encoded ^ bias(tcx, ty)
}

/// The Rust value an encoded case value stands for, sign included.
///
/// What the literal `emit.rs` prints *means*; the printing itself goes through
/// `FnCx::switch_case_value` so that a `SwitchInt` literal is spelled the one way the backend
/// spells integer literals.
pub(crate) fn case_value<'tcx>(tcx: TyCtxt<'tcx>, ty: Ty<'tcx>, encoded: u128) -> i128 {
    let raw = decode(tcx, ty, encoded);
    match value::int_info(tcx, ty) {
        Some((signed, bits)) => value::resign(signed, bits, raw),
        None => raw as i128,
    }
}
