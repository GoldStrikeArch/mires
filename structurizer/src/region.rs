//! The output region tree.

/// A label on a [`Region::Labeled`], [`Region::Loop`], [`Region::Switch`] or
/// [`Region::Dispatch`].
///
/// Labels are only ever attached to a construct some jump names, so a backend
/// can print every one it sees.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Label(pub u32);

/// A compiler introduced variable: a bound scrutinee or a dispatch selector.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TmpId(pub u32);

/// The value an [`Region::If`] or [`Region::Switch`] branches on.
#[derive(Debug)]
pub enum Scrut<C> {
    /// The condition payload itself, evaluated in place. Used when the value
    /// is needed exactly once.
    Payload(C),
    /// A temporary bound by an enclosing [`Region::LetTmp`], read as many
    /// times as the tests need.
    Tmp(TmpId),
}

/// The comparison an [`Region::If`] applies to its scrutinee.
///
/// The structurizer picks the shape, the backend picks the spelling. Every
/// test is exact: the `else_` arm gets precisely the values the test rejects.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Test {
    /// The scrutinee is true. Only produced for [`crate::Term::Cond`].
    IsTrue,
    /// The scrutinee equals the given value.
    Eq(u128),
    /// The given value is strictly less than the scrutinee.
    Lt(u128),
    /// The given value is less than or equal to the scrutinee.
    Le(u128),
    /// The scrutinee is one of `len` consecutive values starting at `lo`.
    /// `len` is at least one.
    InRange { lo: u128, len: u128 },
}

/// A structured region of a function body.
///
/// The tree maps one to one onto JavaScript statements. Control leaves a
/// region either by falling off its end or through a [`Region::Break`],
/// [`Region::Continue`], [`Region::Return`] or [`Region::Throw`].
#[derive(Debug)]
pub enum Region<S, C> {
    /// Regions run one after another.
    Seq(Vec<Region<S, C>>),
    /// The statements of one input block.
    Block(S),
    /// A labeled block. `Break(Some(label))` from inside jumps past it.
    Labeled { label: Label, body: Box<Region<S, C>> },
    /// An endless loop, left by a break or a jump out of the function.
    Loop { label: Option<Label>, body: Box<Region<S, C>> },
    /// Binds a condition payload to a temporary.
    ///
    /// A temporary that no later region reads still evaluates its payload; the
    /// backend may drop the binding when the payload is pure.
    LetTmp { tmp: TmpId, value: C },
    /// A two way branch.
    If {
        scrut: Scrut<C>,
        test: Test,
        then_: Box<Region<S, C>>,
        else_: Box<Region<S, C>>,
    },
    /// A multi way branch on an integer.
    ///
    /// Arms run in order and fall through into each other exactly as in
    /// JavaScript: an arm that can reach its end is terminated by an explicit
    /// `Break(None)`, and `default` is the last arm textually.
    Switch {
        scrut: Scrut<C>,
        arms: Vec<(Vec<u128>, Region<S, C>)>,
        default: Box<Region<S, C>>,
        label: Option<Label>,
    },
    /// A selector driven loop, standing in for a tower of labeled blocks that
    /// would nest too deep.
    ///
    /// Renders as `for (;;) { switch (sel) { case k: ... } [break;] }`. Cases
    /// fall through into each other, `trailing_break` says whether the break
    /// after the switch is reachable and has to be emitted.
    Dispatch {
        sel: TmpId,
        label: Option<Label>,
        cases: Vec<(u32, Region<S, C>)>,
        trailing_break: bool,
    },
    /// Assigns a dispatch selector. The assignment that immediately precedes
    /// a [`Region::Dispatch`] is that selector's declaration site.
    SetSel(TmpId, u32),
    /// Leaves the named construct, or the innermost enclosing loop or switch.
    Break(Option<Label>),
    /// Restarts the named loop, or the innermost enclosing loop.
    Continue(Option<Label>),
    /// Leaves the function normally.
    Return,
    /// Control never gets here.
    Unreachable,
    /// Leaves the function by throwing.
    Throw,
}

impl<S, C> Region<S, C> {
    /// Wraps a list of regions, collapsing the single element case.
    pub(crate) fn seq(mut v: Vec<Region<S, C>>) -> Region<S, C> {
        if v.len() == 1 {
            v.pop().unwrap_or(Region::Seq(Vec::new()))
        } else {
            Region::Seq(v)
        }
    }
}
