//! Turns a control flow graph into structured control flow.
//!
//! [`structure`] takes a [`Cfg`] of basic blocks and returns a [`Region`]
//! tree: loops, labeled blocks, ifs and switches, in the shape a JavaScript
//! backend can print directly. It is a port of the relooper js_of_ocaml uses,
//! with its decision tree construction for multi way branches.
//!
//! ```
//! use structurizer::{structure, Block, BlockId, Cfg, Options, Term};
//!
//! let cfg = Cfg {
//!     entry: BlockId(0),
//!     blocks: vec![
//!         Block { stmts: "x = 1;", term: Term::Goto(BlockId(1)), unwind: None },
//!         Block { stmts: "y = 2;", term: Term::Return, unwind: None },
//!     ],
//! };
//! let out = structure::<&str, &str>(cfg, &Options::default()).unwrap();
//! assert!(!out.stats.irreducible);
//! ```
//!
//! # Input
//!
//! Every successor named by a terminator has to be an index into
//! [`Cfg::blocks`], and the entry has to be one too. Nothing else is required:
//! blocks may come in any order, may be unreachable, may loop back on
//! themselves, and the graph may be irreducible.
//!
//! Blocks not reachable from the entry are dropped. The two payload types are
//! moved into the result untouched, except that statement payloads are asked
//! for a [`Payload::cost`] to decide how much code is worth duplicating.
//!
//! [`Block::unwind`] is reserved for a later pass that turns unwind edges into
//! `try`/`catch`. It takes no part here: it is not an edge of the graph and it
//! does not constrain where a block is placed.
//!
//! # Output
//!
//! Each reachable block contributes exactly one [`Region::Block`], holding its
//! statements, and each branching block contributes one scrutinee, either
//! inline or bound by a [`Region::LetTmp`]. The scrutinee is evaluated once
//! however many times the tests read it.
//!
//! Labels are already resolved. A [`Region::Break`] or [`Region::Continue`]
//! carries a label only where an unlabeled one would land somewhere else, and
//! a construct carries a label only where some jump names it: an unused
//! [`Region::Labeled`] is dropped rather than emitted empty. A backend can
//! print every label it sees and never has to work out which are needed.
//!
//! The `default` of a [`Term::Switch`] takes every value the `cases` do not
//! list, so the tests built from it stay exact. A caller that knows the
//! unlisted values cannot occur, as for a match whose otherwise arm is
//! unreachable, gets better code by passing the widest arm as `default` and
//! leaving that arm's values out of `cases`.
//!
//! # Failure
//!
//! [`Err`] means the input was not a graph this crate can read: an out of
//! range block id, or an empty body. Malformed input never panics.
//!
//! Control flow this crate cannot express as loops and labeled blocks is not
//! an error. An irreducible graph, or one that nests deeper than a few hundred
//! levels, comes back as a single [`Region::Dispatch`] over all its blocks,
//! which expresses any graph at the cost of readability.
//! [`StructureStats::dispatch_fallbacks`] counts those, so a backend can
//! measure how often it happens.

mod cfg;
mod dom;
mod dtree;
mod emit;
mod loops;
mod order;
mod region;

pub use cfg::*;
pub use region::*;

use std::fmt;

/// Knobs on the shape of the output.
#[derive(Clone, Debug)]
pub struct Options {
    /// How many sibling scopes may nest as labeled blocks before the whole
    /// group turns into one [`Region::Dispatch`].
    pub merge_node_max: usize,
    /// How many cases a [`Region::Switch`] may hold before the values are
    /// split by comparisons instead.
    pub switch_max_case: usize,
    /// How much code may be duplicated to keep a loop continuation inside the
    /// loop, in [`Payload::cost`] units.
    pub small_limit: i32,
    /// Emit the whole body as one [`Region::Dispatch`], whatever its shape.
    pub force_dispatch: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            merge_node_max: 10,
            switch_max_case: 60,
            small_limit: 20,
            force_dispatch: false,
        }
    }
}

/// What the input looked like, for reporting and for measuring how often the
/// fallbacks are needed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StructureStats {
    /// The graph has a loop with more than one entry.
    pub irreducible: bool,
    /// How many [`Region::Dispatch`] regions the output holds.
    pub dispatch_fallbacks: u32,
}

/// The result of [`structure`].
#[derive(Debug)]
pub struct Structured<S, C> {
    /// The body, with every payload of the input moved into it.
    pub region: Region<S, C>,
    /// What the input looked like on the way through.
    pub stats: StructureStats,
}

/// A graph this crate cannot read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StructureError {
    /// The body has no blocks, or the entry is not one of them.
    EmptyCfg,
    /// A terminator names a block that does not exist.
    BlockOutOfRange { from: Option<BlockId>, target: BlockId },
    /// A block that is not reachable from the entry was asked about.
    UnreachableBlock(BlockId),
    /// A block was placed in two different scopes.
    BlockCompiledTwice(BlockId),
    /// Regions nested past what a backend can print.
    TooDeep,
    /// An invariant of the algorithm did not hold.
    Internal(&'static str),
}

impl fmt::Display for StructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StructureError::EmptyCfg => write!(f, "the control flow graph has no entry block"),
            StructureError::BlockOutOfRange { from, target } => match from {
                Some(from) => write!(f, "block {} branches to unknown block {}", from.0, target.0),
                None => write!(f, "unknown block {}", target.0),
            },
            StructureError::UnreachableBlock(b) => write!(f, "block {} is not reachable", b.0),
            StructureError::BlockCompiledTwice(b) => write!(f, "block {} was placed twice", b.0),
            StructureError::TooDeep => write!(f, "control flow nests too deeply"),
            StructureError::Internal(m) => write!(f, "internal error: {m}"),
        }
    }
}

impl std::error::Error for StructureError {}

/// How much code a statement payload stands for.
///
/// Only the ratio to [`Options::small_limit`] matters: it decides whether a
/// loop continuation is small enough to be duplicated into the loop rather
/// than placed after it. Anything that must never be duplicated, like a
/// nested function, should report a cost above the limit.
pub trait Payload {
    fn cost(&self) -> i32;
}

impl Payload for () {
    fn cost(&self) -> i32 {
        0
    }
}

impl Payload for str {
    /// One unit per `;`.
    fn cost(&self) -> i32 {
        self.matches(';').count() as i32
    }
}

impl Payload for String {
    fn cost(&self) -> i32 {
        str::cost(self)
    }
}

impl<T: Payload + ?Sized> Payload for &T {
    fn cost(&self) -> i32 {
        T::cost(self)
    }
}

impl<T: Payload> Payload for Vec<T> {
    fn cost(&self) -> i32 {
        self.iter().map(T::cost).sum()
    }
}

/// Turns a control flow graph into a region tree.
///
/// See the crate documentation for what the input has to look like and what
/// the output guarantees.
pub fn structure<S: Payload, C>(
    cfg: Cfg<S, C>,
    opts: &Options,
) -> Result<Structured<S, C>, StructureError> {
    let mut order = order::Order::build(&cfg)?;
    let dom = dom::Dom::build(&order)?;
    let mut stats = StructureStats::default();
    stats.irreducible = !loops::is_reducible(&order, &dom);

    let (skeleton, stats) = if stats.irreducible || opts.force_dispatch {
        let mut emitter = emit::Emitter::new(&cfg, &order, &dom, opts, stats);
        let skeleton = emitter.run_dispatch()?;
        (skeleton, emitter.stats())
    } else {
        // Hoisting a loop continuation out changes which blocks dominate it,
        // so the tree has to be built again afterwards.
        loops::shrink_loops(&cfg, &mut order, opts)?;
        let dom = dom::Dom::build(&order)?;
        let mut emitter = emit::Emitter::new(&cfg, &order, &dom, opts, stats);
        match emitter.run() {
            Ok(skeleton) => (skeleton, emitter.stats()),
            Err(_) => {
                // Nothing has been consumed yet, so a shape the nested form
                // cannot take still has the flat one to fall back on.
                let mut emitter = emit::Emitter::new(&cfg, &order, &dom, opts, stats);
                let skeleton = emitter.run_dispatch()?;
                (skeleton, emitter.stats())
            }
        }
    };

    let region = fill(skeleton, cfg)?;
    Ok(Structured { region, stats })
}

/// Moves the payloads of the graph into the skeleton emission produced.
fn fill<S, C>(skeleton: emit::Skeleton, cfg: Cfg<S, C>) -> Result<Region<S, C>, StructureError> {
    let mut stmts: Vec<Option<S>> = Vec::with_capacity(cfg.blocks.len());
    let mut conds: Vec<Option<C>> = Vec::with_capacity(cfg.blocks.len());
    for b in cfg.blocks {
        stmts.push(Some(b.stmts));
        conds.push(b.term.into_condition());
    }
    fill_region(skeleton, &mut stmts, &mut conds)
}

fn fill_region<S, C>(
    r: emit::Skeleton,
    stmts: &mut [Option<S>],
    conds: &mut [Option<C>],
) -> Result<Region<S, C>, StructureError> {
    Ok(match r {
        Region::Seq(v) => Region::Seq(
            v.into_iter()
                .map(|r| fill_region(r, stmts, conds))
                .collect::<Result<_, _>>()?,
        ),
        Region::Block(b) => Region::Block(take_stmts(stmts, b)?),
        Region::Labeled { label, body } => Region::Labeled {
            label,
            body: Box::new(fill_region(*body, stmts, conds)?),
        },
        Region::Loop { label, body } => Region::Loop {
            label,
            body: Box::new(fill_region(*body, stmts, conds)?),
        },
        Region::LetTmp { tmp, value } => Region::LetTmp { tmp, value: take_cond(conds, value)? },
        Region::If { scrut, test, then_, else_ } => Region::If {
            scrut: fill_scrut(conds, scrut)?,
            test,
            then_: Box::new(fill_region(*then_, stmts, conds)?),
            else_: Box::new(fill_region(*else_, stmts, conds)?),
        },
        Region::Switch { scrut, arms, default, label } => Region::Switch {
            scrut: fill_scrut(conds, scrut)?,
            arms: arms
                .into_iter()
                .map(|(v, r)| Ok((v, fill_region(r, stmts, conds)?)))
                .collect::<Result<_, StructureError>>()?,
            default: Box::new(fill_region(*default, stmts, conds)?),
            label,
        },
        Region::Dispatch { sel, label, cases, trailing_break } => Region::Dispatch {
            sel,
            label,
            cases: cases
                .into_iter()
                .map(|(c, r)| Ok((c, fill_region(r, stmts, conds)?)))
                .collect::<Result<_, StructureError>>()?,
            trailing_break,
        },
        Region::SetSel(t, c) => Region::SetSel(t, c),
        Region::Break(l) => Region::Break(l),
        Region::Continue(l) => Region::Continue(l),
        Region::Return => Region::Return,
        Region::Unreachable => Region::Unreachable,
        Region::Throw => Region::Throw,
    })
}

fn take_stmts<S>(stmts: &mut [Option<S>], b: BlockId) -> Result<S, StructureError> {
    stmts
        .get_mut(b.index())
        .and_then(Option::take)
        .ok_or(StructureError::BlockCompiledTwice(b))
}

fn take_cond<C>(conds: &mut [Option<C>], b: BlockId) -> Result<C, StructureError> {
    conds
        .get_mut(b.index())
        .and_then(Option::take)
        .ok_or(StructureError::Internal("scrutinee read more than once"))
}

fn fill_scrut<C>(
    conds: &mut [Option<C>],
    s: Scrut<BlockId>,
) -> Result<Scrut<C>, StructureError> {
    match s {
        Scrut::Payload(b) => take_cond(conds, b).map(Scrut::Payload),
        Scrut::Tmp(t) => Ok(Scrut::Tmp(t)),
    }
}
