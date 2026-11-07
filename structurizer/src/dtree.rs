//! Decision trees for conditional and multi way terminators.
//!
//! A tree is built once per branching block: first to count how many of its
//! leaves reach a given target, then to drive the emission. Every test is
//! exact, so the tree can be read as a partition of the scrutinee's values.

use crate::cfg::BlockId;
use crate::region::Test;
use crate::Options;

/// A decision tree over the values of one scrutinee.
#[derive(Debug)]
pub(crate) enum DTree {
    /// Every remaining value goes to this block.
    Branch(BlockId),
    /// `then_` takes the values the test accepts, `else_` the rest.
    If(Test, Box<DTree>, Box<DTree>),
    /// A jump table. `default` takes every value no arm lists.
    Switch { arms: Vec<(Vec<u128>, BlockId)>, default: BlockId },
}

impl DTree {
    /// How many leaves of the tree reach a block.
    ///
    /// Two or more means the block cannot simply be placed after the branch;
    /// it needs a scope of its own that each leaf jumps to.
    pub(crate) fn nbbranch(&self, target: BlockId) -> u32 {
        match self {
            DTree::Branch(b) => u32::from(*b == target),
            DTree::If(_, a, b) => a.nbbranch(target) + b.nbbranch(target),
            DTree::Switch { arms, default } => {
                arms.iter().filter(|(_, b)| *b == target).count() as u32
                    + u32::from(*default == target)
            }
        }
    }

    /// How many times the tree inspects the scrutinee.
    ///
    /// More than once means the scrutinee has to be bound to a temporary first.
    pub(crate) fn nbcomp(&self) -> u32 {
        match self {
            DTree::Branch(_) => 0,
            DTree::If(_, a, b) => 1 + a.nbcomp() + b.nbcomp(),
            DTree::Switch { .. } => 1,
        }
    }
}

/// The tree of a two way branch.
pub(crate) fn build_if(then_: BlockId, else_: BlockId) -> DTree {
    DTree::If(
        Test::IsTrue,
        Box::new(DTree::Branch(then_)),
        Box::new(DTree::Branch(else_)),
    )
}

/// A maximal run of consecutive values that share a target.
#[derive(Clone, Debug)]
struct Run {
    lo: u128,
    hi: u128,
    target: BlockId,
}

impl Run {
    fn count(&self) -> u128 {
        self.hi - self.lo + 1
    }
}

/// Runs merged by target, ordered so the last one is the widest.
#[derive(Clone, Debug)]
struct Group {
    ranges: Vec<(u128, u128)>,
    target: BlockId,
    /// Number of values, or `None` when the group is the fallback and so
    /// covers every value no other group lists.
    count: Option<u128>,
}

impl Group {
    fn values(&self) -> Vec<u128> {
        let mut v = Vec::new();
        for &(lo, hi) in &self.ranges {
            let mut x = lo;
            loop {
                v.push(x);
                if x == hi {
                    break;
                }
                x += 1;
            }
        }
        v
    }
}

/// The tree of a multi way branch.
///
/// `cases` may be in any order and may repeat a value; the first target listed
/// for a value wins.
pub(crate) fn build_switch(
    cases: &[(u128, BlockId)],
    default: BlockId,
    opts: &Options,
) -> DTree {
    let mut sorted: Vec<(u128, BlockId)> = cases.to_vec();
    sorted.sort_by_key(|(v, _)| *v);
    sorted.dedup_by_key(|(v, _)| *v);

    // Group the contiguous cases that share a target.
    let mut runs: Vec<Run> = Vec::new();
    for (v, t) in sorted {
        match runs.last_mut() {
            Some(r) if r.target == t && r.hi + 1 == v => r.hi = v,
            _ => runs.push(Run { lo: v, hi: v, target: t }),
        }
    }
    if runs.is_empty() {
        return DTree::Branch(default);
    }
    build(&runs, default, None, None, opts)
}

/// Builds the tree for the runs inside the value interval `[lo, hi]`, where
/// `None` means unbounded. Values in the interval that no run lists go to
/// `default`.
fn build(
    runs: &[Run],
    default: BlockId,
    lo: Option<u128>,
    hi: Option<u128>,
    opts: &Options,
) -> DTree {
    let groups = normalize(runs, default, lo, hi);
    match groups.len() {
        0 => return DTree::Branch(default),
        // One group means it either is the fallback or covers the whole
        // interval; nothing is left to test.
        1 => return DTree::Branch(groups[0].target),
        2 => {
            let (a, b) = (&groups[0], &groups[1]);
            let full = covers(&groups, lo, hi);
            if a.count == Some(1) {
                return DTree::If(
                    Test::Eq(a.ranges[0].0),
                    Box::new(DTree::Branch(a.target)),
                    Box::new(DTree::Branch(b.target)),
                );
            }
            if full && b.count == Some(1) {
                return DTree::If(
                    Test::Eq(b.ranges[0].0),
                    Box::new(DTree::Branch(b.target)),
                    Box::new(DTree::Branch(a.target)),
                );
            }
            if full {
                if let (Some(&(lo1, hi1)), Some(&(lo2, hi2))) = (one_range(a), one_range(b)) {
                    if hi1 < lo2 {
                        return DTree::If(
                            Test::Lt(hi1),
                            Box::new(DTree::Branch(b.target)),
                            Box::new(DTree::Branch(a.target)),
                        );
                    }
                    if hi2 < lo1 {
                        return DTree::If(
                            Test::Lt(hi2),
                            Box::new(DTree::Branch(a.target)),
                            Box::new(DTree::Branch(b.target)),
                        );
                    }
                }
            }
            if let Some(t) = single_test(a) {
                return DTree::If(
                    t,
                    Box::new(DTree::Branch(a.target)),
                    Box::new(DTree::Branch(b.target)),
                );
            }
        }
        _ => {}
    }

    // A jump table, unless it would be too wide to be worth one.
    let last = groups.len() - 1;
    let nbcases: u128 = groups[..last]
        .iter()
        .map(|g| g.count.unwrap_or(0))
        .sum::<u128>()
        + 1;
    if nbcases <= opts.switch_max_case as u128 || runs.len() < 2 {
        return DTree::Switch {
            arms: groups[..last].iter().map(|g| (g.values(), g.target)).collect(),
            default: groups[last].target,
        };
    }

    // Split the runs in half and test which half the value falls in.
    let h = (runs.len() - 1) / 2;
    let pivot = runs[h + 1].lo;
    let left = build(&runs[..=h], default, lo, Some(pivot - 1), opts);
    let right = build(&runs[h + 1..], default, Some(pivot), hi, opts);
    DTree::If(Test::Le(pivot), Box::new(right), Box::new(left))
}

/// Merges runs by target and orders the groups by width, widest last, so the
/// widest becomes the `else` or the `default` and needs no test of its own.
///
/// The fallback joins in as a group of unbounded width unless the runs already
/// cover the whole interval.
fn normalize(runs: &[Run], default: BlockId, lo: Option<u128>, hi: Option<u128>) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    for r in runs {
        match groups.iter_mut().find(|g| g.target == r.target) {
            Some(g) => {
                g.ranges.push((r.lo, r.hi));
                g.count = Some(g.count.unwrap_or(0) + r.count());
            }
            None => groups.push(Group {
                ranges: vec![(r.lo, r.hi)],
                target: r.target,
                count: Some(r.count()),
            }),
        }
    }
    let listed: u128 = groups.iter().map(|g| g.count.unwrap_or(0)).sum();
    if !interval_covered(listed, lo, hi) {
        match groups.iter_mut().find(|g| g.target == default) {
            Some(g) => g.count = None,
            None => groups.push(Group { ranges: Vec::new(), target: default, count: None }),
        }
    }
    groups.sort_by(|a, b| match (a.count, b.count) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, _) => std::cmp::Ordering::Greater,
        (_, None) => std::cmp::Ordering::Less,
        (Some(x), Some(y)) => x
            .cmp(&y)
            .then_with(|| a.ranges[0].0.cmp(&b.ranges[0].0))
            .then_with(|| a.target.cmp(&b.target)),
    });
    groups
}

/// Whether the groups leave no value of the interval to the fallback.
fn covers(groups: &[Group], lo: Option<u128>, hi: Option<u128>) -> bool {
    let listed: u128 = groups.iter().map(|g| g.count.unwrap_or(0)).sum();
    groups.iter().all(|g| g.count.is_some()) && interval_covered(listed, lo, hi)
}

fn interval_covered(listed: u128, lo: Option<u128>, hi: Option<u128>) -> bool {
    match (lo, hi) {
        (Some(lo), Some(hi)) if hi >= lo => match hi.checked_sub(lo).and_then(|d| d.checked_add(1)) {
            Some(width) => listed >= width,
            None => false,
        },
        _ => false,
    }
}

fn one_range(g: &Group) -> Option<&(u128, u128)> {
    match g.ranges.as_slice() {
        [r] => Some(r),
        _ => None,
    }
}

/// The exact test for a group made of one run of values.
fn single_test(g: &Group) -> Option<Test> {
    let &(lo, hi) = one_range(g)?;
    if lo == hi {
        Some(Test::Eq(lo))
    } else {
        Some(Test::InRange { lo, len: hi - lo + 1 })
    }
}
