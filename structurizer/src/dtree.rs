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
    Switch {
        arms: Vec<(Vec<u128>, BlockId)>,
        default: BlockId,
    },
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

/// Number of `u128` values in a set of disjoint ranges.
///
/// The full `u128` domain has `2^128` values, which is one larger than
/// `u128::MAX` and therefore cannot be represented as a plain `u128`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SpanCount {
    /// Strictly fewer than `2^128` values.
    Exact(u128),
    /// All `2^128` values (`0..=u128::MAX`).
    FullDomain,
}

impl SpanCount {
    /// Number of values in the inclusive range `lo..=hi`.
    fn from_range(lo: u128, hi: u128) -> SpanCount {
        match hi.checked_sub(lo) {
            None => SpanCount::Exact(0),
            Some(diff) => match diff.checked_add(1) {
                Some(n) => SpanCount::Exact(n),
                None => SpanCount::FullDomain,
            },
        }
    }

    /// Sum of counts of two disjoint subsets of `u128`.
    fn add(self, other: SpanCount) -> SpanCount {
        match (self, other) {
            (SpanCount::Exact(a), SpanCount::Exact(b)) => match a.checked_add(b) {
                Some(sum) => SpanCount::Exact(sum),
                None => SpanCount::FullDomain,
            },
            _ => SpanCount::FullDomain,
        }
    }
}

/// A maximal run of consecutive values that share a target.
#[derive(Clone, Debug)]
struct Run {
    lo: u128,
    hi: u128,
    target: BlockId,
}

impl Run {
    fn count(&self) -> SpanCount {
        SpanCount::from_range(self.lo, self.hi)
    }
}

/// Whether a group owns an explicit non-empty list of ranges or catches every
/// remaining value as the fallback.
#[derive(Clone, Debug)]
enum GroupKind {
    Bounded {
        first: (u128, u128),
        rest: Vec<(u128, u128)>,
        count: SpanCount,
    },
    Fallback,
}

/// Runs merged by target, ordered so the widest (or fallback) is last.
#[derive(Clone, Debug)]
struct Group {
    target: BlockId,
    kind: GroupKind,
}

impl Group {
    fn bounded(run: &Run) -> Group {
        Group {
            target: run.target,
            kind: GroupKind::Bounded {
                first: (run.lo, run.hi),
                rest: Vec::new(),
                count: run.count(),
            },
        }
    }

    fn fallback(target: BlockId) -> Group {
        Group {
            target,
            kind: GroupKind::Fallback,
        }
    }

    fn push_run(&mut self, run: &Run) {
        if let GroupKind::Bounded { rest, count, .. } = &mut self.kind {
            rest.push((run.lo, run.hi));
            *count = count.add(run.count());
        }
    }

    fn count(&self) -> Option<SpanCount> {
        match self.kind {
            GroupKind::Bounded { count, .. } => Some(count),
            GroupKind::Fallback => None,
        }
    }

    fn single_range(&self) -> Option<(u128, u128)> {
        match &self.kind {
            GroupKind::Bounded { first, rest, .. } if rest.is_empty() => Some(*first),
            _ => None,
        }
    }

    fn single_value(&self) -> Option<u128> {
        self.single_range()
            .and_then(|(lo, hi)| (lo == hi).then_some(lo))
    }

    fn values(&self) -> Vec<u128> {
        let GroupKind::Bounded { first, rest, .. } = &self.kind else {
            return Vec::new();
        };
        let mut v = Vec::new();
        for &(lo, hi) in std::iter::once(first).chain(rest.iter()) {
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
pub(crate) fn build_switch(cases: &[(u128, BlockId)], default: BlockId, opts: &Options) -> DTree {
    let mut sorted: Vec<(u128, BlockId)> = cases.to_vec();
    sorted.sort_by_key(|(v, _)| *v);
    sorted.dedup_by_key(|(v, _)| *v);

    // Group the contiguous cases that share a target.
    let mut runs: Vec<Run> = Vec::new();
    for (v, t) in sorted {
        match runs.last_mut() {
            Some(r) if r.target == t && r.hi.checked_add(1) == Some(v) => r.hi = v,
            _ => runs.push(Run {
                lo: v,
                hi: v,
                target: t,
            }),
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
            if let Some(v) = a.single_value() {
                return DTree::If(
                    Test::Eq(v),
                    Box::new(DTree::Branch(a.target)),
                    Box::new(DTree::Branch(b.target)),
                );
            }
            if full {
                if let Some(v) = b.single_value() {
                    return DTree::If(
                        Test::Eq(v),
                        Box::new(DTree::Branch(b.target)),
                        Box::new(DTree::Branch(a.target)),
                    );
                }
                if let (Some((lo1, hi1)), Some((lo2, hi2))) = (a.single_range(), b.single_range()) {
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
    let nbcases: u128 = groups[..last].iter().fold(1u128, |acc, g| {
        let count = match g.count() {
            Some(SpanCount::Exact(n)) => n,
            Some(SpanCount::FullDomain) | None => u128::MAX,
        };
        acc.saturating_add(count)
    });
    if nbcases <= opts.switch_max_case as u128 || runs.len() < 2 {
        return DTree::Switch {
            arms: groups[..last]
                .iter()
                .map(|g| (g.values(), g.target))
                .collect(),
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
            Some(g) => g.push_run(r),
            None => groups.push(Group::bounded(r)),
        }
    }
    let listed = groups.iter().fold(SpanCount::Exact(0), |acc, g| {
        g.count().map_or(acc, |c| acc.add(c))
    });
    if !interval_covered(listed, lo, hi) {
        match groups.iter_mut().find(|g| g.target == default) {
            Some(g) => g.kind = GroupKind::Fallback,
            None => groups.push(Group::fallback(default)),
        }
    }
    groups.sort_by(|a, b| match (&a.kind, &b.kind) {
        (GroupKind::Fallback, GroupKind::Fallback) => std::cmp::Ordering::Equal,
        (GroupKind::Fallback, GroupKind::Bounded { .. }) => std::cmp::Ordering::Greater,
        (GroupKind::Bounded { .. }, GroupKind::Fallback) => std::cmp::Ordering::Less,
        (
            GroupKind::Bounded {
                first: first_a,
                count: count_a,
                ..
            },
            GroupKind::Bounded {
                first: first_b,
                count: count_b,
                ..
            },
        ) => count_a
            .cmp(count_b)
            .then_with(|| first_a.0.cmp(&first_b.0))
            .then_with(|| a.target.cmp(&b.target)),
    });
    groups
}

/// Whether the groups leave no value of the interval to the fallback.
fn covers(groups: &[Group], lo: Option<u128>, hi: Option<u128>) -> bool {
    let mut listed = SpanCount::Exact(0);
    for g in groups {
        let Some(c) = g.count() else {
            return false;
        };
        listed = listed.add(c);
    }
    interval_covered(listed, lo, hi)
}

fn interval_covered(listed: SpanCount, lo: Option<u128>, hi: Option<u128>) -> bool {
    if listed == SpanCount::FullDomain {
        return true;
    }
    match (lo, hi) {
        (Some(lo), Some(hi)) if hi >= lo => listed >= SpanCount::from_range(lo, hi),
        _ => false,
    }
}

/// The exact test for a group made of one run of values.
///
/// Returns `None` for the fallback, for multi-run groups, or for the full
/// `u128` domain `0..=u128::MAX` (which needs no test).
fn single_test(g: &Group) -> Option<Test> {
    let (lo, hi) = g.single_range()?;
    if lo == hi {
        Some(Test::Eq(lo))
    } else if hi == u128::MAX {
        (lo > 0).then_some(Test::Le(lo))
    } else {
        Some(Test::InRange {
            lo,
            len: hi - lo + 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(tree: &DTree, v: u128) -> BlockId {
        match tree {
            DTree::Branch(b) => *b,
            DTree::If(test, then_, else_) => {
                let holds = match *test {
                    Test::IsTrue => v != 0,
                    Test::Eq(n) => v == n,
                    Test::Lt(n) => n < v,
                    Test::Le(n) => n <= v,
                    Test::InRange { lo, len } => v >= lo && v - lo < len,
                };
                if holds {
                    eval(then_, v)
                } else {
                    eval(else_, v)
                }
            }
            DTree::Switch { arms, default } => arms
                .iter()
                .find(|(vs, _)| vs.contains(&v))
                .map_or(*default, |(_, b)| *b),
        }
    }

    #[test]
    fn full_u128_domain_run_does_not_overflow() {
        let run = Run {
            lo: 0,
            hi: u128::MAX,
            target: BlockId(1),
        };
        assert_eq!(run.count(), SpanCount::FullDomain);
        assert_eq!(single_test(&Group::bounded(&run)), None);

        let opts = Options::default();
        for (lo, hi) in [(None, None), (Some(0), Some(u128::MAX))] {
            let tree = build(std::slice::from_ref(&run), BlockId(2), lo, hi, &opts);
            assert!(matches!(tree, DTree::Branch(BlockId(1))));
            assert_eq!(eval(&tree, 0), BlockId(1));
            assert_eq!(eval(&tree, u128::MAX), BlockId(1));
        }
    }

    #[test]
    fn upper_u128_max_boundary_uses_le_without_overflow() {
        let opts = Options::default();
        let tree = build_switch(
            &[(u128::MAX - 1, BlockId(1)), (u128::MAX, BlockId(1))],
            BlockId(2),
            &opts,
        );
        assert!(matches!(
            tree,
            DTree::If(Test::Le(lo), _, _) if lo == u128::MAX - 1
        ));
        assert_eq!(eval(&tree, 0), BlockId(2));
        assert_eq!(eval(&tree, u128::MAX - 2), BlockId(2));
        assert_eq!(eval(&tree, u128::MAX - 1), BlockId(1));
        assert_eq!(eval(&tree, u128::MAX), BlockId(1));

        let single = build_switch(&[(u128::MAX, BlockId(1))], BlockId(2), &opts);
        assert!(matches!(single, DTree::If(Test::Eq(u128::MAX), _, _)));
        assert_eq!(eval(&single, u128::MAX - 1), BlockId(2));
        assert_eq!(eval(&single, u128::MAX), BlockId(1));
    }

    #[test]
    fn disjoint_runs_spanning_full_u128_domain_do_not_overflow() {
        let opts = Options::default();
        let runs = [
            Run {
                lo: 0,
                hi: 5,
                target: BlockId(1),
            },
            Run {
                lo: 6,
                hi: u128::MAX,
                target: BlockId(2),
            },
        ];
        let tree = build(&runs, BlockId(3), None, None, &opts);
        assert!(matches!(tree, DTree::If(Test::Lt(5), _, _)));
        assert_eq!(eval(&tree, 0), BlockId(1));
        assert_eq!(eval(&tree, 5), BlockId(1));
        assert_eq!(eval(&tree, 6), BlockId(2));
        assert_eq!(eval(&tree, u128::MAX), BlockId(2));
    }

    #[test]
    fn near_full_u128_runs_with_gap_do_not_overflow_nbcases() {
        let opts = Options::default();
        let runs = [
            Run {
                lo: 0,
                hi: u128::MAX - 2,
                target: BlockId(1),
            },
            Run {
                lo: u128::MAX,
                hi: u128::MAX,
                target: BlockId(2),
            },
        ];
        let tree = build(&runs, BlockId(3), None, None, &opts);
        assert_eq!(eval(&tree, 0), BlockId(1));
        assert_eq!(eval(&tree, u128::MAX - 2), BlockId(1));
        assert_eq!(eval(&tree, u128::MAX - 1), BlockId(3));
        assert_eq!(eval(&tree, u128::MAX), BlockId(2));
    }
}
