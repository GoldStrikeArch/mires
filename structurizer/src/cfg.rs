//! The input control flow graph.

/// Index of a block in [`Cfg::blocks`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct BlockId(pub u32);

impl BlockId {
    /// The index into [`Cfg::blocks`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A basic block: a straight line run of statements ended by a terminator.
///
/// `S` is the statement payload and `C` the condition payload. Neither is
/// inspected beyond [`crate::Payload::cost`]; both are moved into the result.
#[derive(Debug)]
pub struct Block<S, C> {
    /// Everything that runs before the terminator.
    pub stmts: S,
    /// How the block leaves.
    pub term: Term<C>,
    /// The handler an unwinding call inside this block transfers to.
    ///
    /// Reserved for a later try/catch pass. The algorithm ignores it: it is
    /// neither an edge of the graph nor a constraint on block placement.
    pub unwind: Option<BlockId>,
}

/// How a block transfers control.
#[derive(Debug)]
pub enum Term<C> {
    /// Unconditional branch.
    Goto(BlockId),
    /// Branch on a truth value.
    Cond { on: C, then_: BlockId, else_: BlockId },
    /// Branch on an integer value, `default` taking every unlisted value.
    Switch { on: C, cases: Vec<(u128, BlockId)>, default: BlockId },
    /// Leave the function normally.
    Return,
    /// Control never gets here.
    Unreachable,
    /// Leave the function by throwing.
    Throw,
}

impl<C> Term<C> {
    /// The blocks this terminator can transfer to, in no particular order and
    /// possibly with repeats.
    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            Term::Goto(b) => vec![*b],
            Term::Cond { then_, else_, .. } => vec![*then_, *else_],
            Term::Switch { cases, default, .. } => {
                let mut v: Vec<BlockId> = cases.iter().map(|(_, b)| *b).collect();
                v.push(*default);
                v
            }
            Term::Return | Term::Unreachable | Term::Throw => Vec::new(),
        }
    }

    /// The condition payload, for terminators that branch on a value.
    pub fn into_condition(self) -> Option<C> {
        match self {
            Term::Cond { on, .. } | Term::Switch { on, .. } => Some(on),
            _ => None,
        }
    }
}

/// A function body as a graph of blocks.
#[derive(Debug)]
pub struct Cfg<S, C> {
    /// All blocks, indexed by [`BlockId`]. Blocks not reachable from `entry`
    /// are dropped by [`crate::structure`].
    pub blocks: Vec<Block<S, C>>,
    /// Where the function starts.
    pub entry: BlockId,
}
