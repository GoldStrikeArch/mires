//! Turning the analysed graph into a region tree.
//!
//! Emission runs on a skeleton whose payloads are block ids, so a failure
//! costs nothing: the caller can fall back to a whole function dispatch and
//! only then move the real payloads in.

use crate::cfg::{BlockId, Cfg, Term};
use crate::dom::Dom;
use crate::dtree::{build_if, build_switch, DTree};
use crate::order::Order;
use crate::region::{Label, Region, Scrut, TmpId};
use crate::{Options, StructureError, StructureStats};

/// A region tree that still refers to blocks instead of holding their
/// payloads. `Block(b)` stands for the statements of `b`, a condition payload
/// for the scrutinee of `b`'s terminator.
pub(crate) type Skeleton = Region<BlockId, BlockId>;

/// Bound on how deeply regions nest. Beyond this the emitted code would be
/// hard on JavaScript parsers, and the recursive walks here would be hard on
/// the stack, so the caller falls back to a dispatch loop.
const MAX_DEPTH: u32 = 300;

/// Where control goes when it falls off the end of the code being emitted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ft {
    /// Out of the function.
    Return,
    /// To a block that is emitted right after.
    Block(BlockId),
}

/// What an enclosing construct does for jumps that name a block.
#[derive(Clone, Copy, Debug)]
enum SKind {
    /// Back to the head of a loop: `continue`.
    Loop,
    /// Past the end of a loop: `break`.
    ExitLoop(u32),
    /// Past the end of a switch: `break`.
    ExitSwitch(u32),
    /// Past the end of a labeled block: `break label`.
    Forward,
    /// Around the dispatch loop: `sel = case; continue`.
    Dispatch { sel: TmpId, case: u32 },
}

/// One entry of the scope stack: the block reached by leaving `label` the way
/// `kind` says.
#[derive(Clone, Copy, Debug)]
struct Scope {
    target: BlockId,
    label: Label,
    kind: SKind,
}

/// Whether a jump is spelled `break` or `continue`, which decides what can
/// capture it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Br {
    Break,
    Continue,
}

pub(crate) struct Emitter<'a, S, C> {
    cfg: &'a Cfg<S, C>,
    order: &'a Order,
    dom: &'a Dom,
    opts: &'a Options,
    stats: StructureStats,
    visited: Vec<bool>,
    /// Set when some jump names the label, so unused ones can be dropped.
    label_used: Vec<bool>,
    /// Set when some jump leaves a loop or a switch, so the code after it is
    /// known to be reachable.
    exit_used: Vec<bool>,
    next_tmp: u32,
    depth: u32,
}

impl<'a, S, C> Emitter<'a, S, C> {
    pub(crate) fn new(
        cfg: &'a Cfg<S, C>,
        order: &'a Order,
        dom: &'a Dom,
        opts: &'a Options,
        stats: StructureStats,
    ) -> Emitter<'a, S, C> {
        Emitter {
            cfg,
            order,
            dom,
            opts,
            stats,
            visited: vec![false; cfg.blocks.len()],
            label_used: Vec::new(),
            exit_used: Vec::new(),
            next_tmp: 0,
            depth: 0,
        }
    }

    pub(crate) fn stats(&self) -> StructureStats {
        self.stats
    }

    /// The whole body as nested loops and labeled blocks.
    pub(crate) fn run(&mut self) -> Result<Skeleton, StructureError> {
        let entry = *self.order.rpo.first().ok_or(StructureError::EmptyCfg)?;
        let mut stack = Vec::new();
        let (_, code) = self.compile_block(entry, &mut stack, Ft::Return)?;
        let skel = Region::seq(code);
        for &b in &self.order.rpo {
            if !self.visited[b.index()] {
                return Err(StructureError::Internal(
                    "a reachable block was not emitted",
                ));
            }
        }
        Ok(self.drop_unused_labels(skel))
    }

    /// The whole body as one flat dispatch loop.
    ///
    /// This is what [`Options::force_dispatch`] asks for and what a body the
    /// nested form fails on falls back to: no block is placed relative to any
    /// other, so nothing has to nest.
    ///
    /// Blocks and cases are not one to one. A block whose one predecessor
    /// hands control over unconditionally is concatenated into that
    /// predecessor's case, and a jump to the case emitted next falls through
    /// instead of assigning the selector, so only the blocks control can reach
    /// from several places pay for a case of their own.
    pub(crate) fn run_dispatch(&mut self) -> Result<Skeleton, StructureError> {
        self.stats.dispatch_fallbacks += 1;
        self.stats.whole_body_dispatch = true;
        let sel = self.fresh_tmp();
        let label = self.fresh_label();

        // The entry is never absorbed: case 0 is where the dispatch starts.
        let mut absorbed = vec![false; self.cfg.blocks.len()];
        for &b in self.order.rpo.iter().skip(1) {
            let [p] = self.order.preds[b.index()][..] else {
                continue;
            };
            let handover = matches!(
                self.cfg.blocks.get(p.index()).map(|block| &block.term),
                Some(Term::Goto(t)) if *t == b
            );
            if p != b && handover {
                absorbed[b.index()] = true;
            }
        }
        let heads: Vec<BlockId> = self
            .order
            .rpo
            .iter()
            .copied()
            .filter(|b| !absorbed[b.index()])
            .collect();

        let mut stack: Vec<Scope> = Vec::new();
        for (i, &b) in heads.iter().enumerate() {
            stack.push(Scope {
                target: b,
                label,
                kind: SKind::Dispatch {
                    sel,
                    case: i as u32,
                },
            });
        }
        let mut cases = Vec::with_capacity(heads.len());
        let mut last_never = true;
        for (i, &head) in heads.iter().enumerate() {
            // Falling off a case runs the one emitted after it, which is what
            // a jump to that case's block means.
            let ft = match heads.get(i + 1) {
                Some(&next) => Ft::Block(next),
                None => Ft::Return,
            };
            let mut body = Vec::new();
            let mut pc = head;
            let never = loop {
                let (stmts, branch) = self.take_block(pc)?;
                body.push(Region::Block(stmts));
                if let Branching::Goto(next) = branch {
                    if absorbed[next.index()] {
                        pc = next;
                        continue;
                    }
                }
                let (never, mut code) = self.compile_branch_or_test(branch, &mut stack, ft)?;
                body.append(&mut code);
                break never;
            };
            if i + 1 == heads.len() {
                last_never = never;
            }
            cases.push((i as u32, Region::seq(body)));
        }
        for &b in &self.order.rpo {
            if !self.visited[b.index()] {
                return Err(StructureError::Internal(
                    "a reachable block was not emitted",
                ));
            }
        }
        let skel = Region::Seq(vec![
            Region::SetSel(sel, 0),
            Region::Dispatch {
                sel,
                label: Some(label),
                cases,
                trailing_break: !last_never,
            },
        ]);
        Ok(self.drop_unused_labels(skel))
    }

    fn fresh_label(&mut self) -> Label {
        self.label_used.push(false);
        Label(self.label_used.len() as u32 - 1)
    }

    fn fresh_exit(&mut self) -> u32 {
        self.exit_used.push(false);
        self.exit_used.len() as u32 - 1
    }

    fn fresh_tmp(&mut self) -> TmpId {
        self.next_tmp += 1;
        TmpId(self.next_tmp - 1)
    }

    fn used(&self, l: Label) -> Option<Label> {
        if self.label_used.get(l.0 as usize).copied().unwrap_or(false) {
            Some(l)
        } else {
            None
        }
    }

    fn take_block(&mut self, pc: BlockId) -> Result<(BlockId, Branching), StructureError> {
        let block = self
            .cfg
            .blocks
            .get(pc.index())
            .ok_or(StructureError::BlockOutOfRange {
                from: None,
                target: pc,
            })?;
        if self.visited[pc.index()] {
            return Err(StructureError::BlockCompiledTwice(pc));
        }
        self.visited[pc.index()] = true;
        let branch = match &block.term {
            Term::Goto(b) => Branching::Goto(*b),
            Term::Return => Branching::Return,
            Term::Unreachable => Branching::Unreachable,
            Term::Throw => Branching::Throw,
            Term::Cond { then_, else_, .. } => Branching::Test(pc, build_if(*then_, *else_)),
            Term::Switch { cases, default, .. } => {
                Branching::Test(pc, build_switch(cases, *default, self.opts))
            }
        };
        Ok((pc, branch))
    }

    /// Whether the block heads a loop the nested form can express: some back
    /// edge lands on it from a block it dominates.
    ///
    /// A retreating edge whose source the block does not dominate is a second
    /// entry into a loop headed elsewhere. A loop wrapped around the block
    /// could never capture that jump, which instead resolves through the
    /// dispatch an enclosing [`Emitter::compile_block_no_loop`] routed the
    /// cycle through.
    fn heads_a_loop(&self, pc: BlockId) -> bool {
        self.order.preds[pc.index()]
            .iter()
            .any(|&p| self.order.is_backward(p, pc) && self.dom.dominates(self.order, pc, p))
    }

    /// Emits a block, wrapping it in a loop when it heads one.
    fn compile_block(
        &mut self,
        pc: BlockId,
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        if !self.heads_a_loop(pc) {
            return self.compile_block_no_loop(pc, stack, ft);
        }
        self.enter()?;
        let label = self.fresh_label();
        let exit = self.fresh_exit();
        let depth = stack.len();
        stack.push(Scope {
            target: pc,
            label,
            kind: SKind::Loop,
        });
        if let Ft::Block(join) = ft {
            stack.push(Scope {
                target: join,
                label,
                kind: SKind::ExitLoop(exit),
            });
        }
        let (never_body, body) = self.compile_block_no_loop(pc, stack, Ft::Block(pc))?;
        stack.truncate(depth);
        self.leave();
        let never = !self.exit_used[exit as usize] && never_body;
        Ok((
            never,
            vec![Region::Loop {
                label: Some(label),
                body: Box::new(Region::seq(body)),
            }],
        ))
    }

    /// Emits a block and everything it dominates that needs a scope.
    fn compile_block_no_loop(
        &mut self,
        pc: BlockId,
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        let (stmts, branch) = self.take_block(pc)?;

        // A block dominated by this one needs a scope of its own when control
        // can reach it from more than one place: several forward edges, or
        // several leaves of this block's decision tree.
        let mut new_scopes: Vec<BlockId> = self.dom.children[pc.index()]
            .iter()
            .copied()
            .filter(|&c| branch.nbbranch(c) >= 2 || self.order.is_merge_node(c))
            .collect();
        // A cycle among sibling subtrees is a loop with more than one entry,
        // which no arrangement of labeled blocks can jump backward into. The
        // siblings on such a cycle all become cases of one dispatch instead,
        // whose selector a jump from any of them can set. Only a graph with a
        // multi entry loop somewhere pays for the search.
        let mut cyclic = Vec::new();
        if self.stats.irreducible {
            cyclic = crate::loops::cyclic_children(self.order, self.dom, pc);
            for &member in &cyclic {
                if !new_scopes.contains(&member) {
                    new_scopes.push(member);
                }
            }
        }
        self.order.sort_in_post_order(&mut new_scopes);

        let (never, mut after) =
            if !cyclic.is_empty() || new_scopes.len() > self.opts.merge_node_max {
                self.compile_dispatch(branch, &new_scopes, stack, ft)?
            } else {
                self.compile_scopes(branch, &new_scopes, stack, ft)?
            };
        let mut code = vec![Region::Block(stmts)];
        code.append(&mut after);
        Ok((never, code))
    }

    /// Nests one labeled block per scope, outermost first, each falling
    /// through into the one emitted before it.
    fn compile_scopes(
        &mut self,
        branch: Branching,
        scopes: &[BlockId],
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        let Some((&x, rest)) = scopes.split_first() else {
            return self.compile_branch_or_test(branch, stack, ft);
        };
        self.enter()?;
        let label = self.fresh_label();
        let depth = stack.len();
        stack.push(Scope {
            target: x,
            label,
            kind: SKind::Forward,
        });
        let (_, inner) = self.compile_scopes(branch, rest, stack, Ft::Block(x))?;
        let (never, mut code) = self.compile_block(x, stack, ft)?;
        stack.truncate(depth);
        self.leave();
        let mut out = vec![Region::Labeled {
            label,
            body: Box::new(Region::seq(inner)),
        }];
        out.append(&mut code);
        Ok((never, out))
    }

    /// Emits the scopes as the cases of one selector driven loop.
    ///
    /// A tower of labeled blocks as deep as `scopes` overflows some JavaScript
    /// parsers, so past a threshold the same layout is expressed flatly: the
    /// cases keep the textual order of the nested form, so a jump to the next
    /// case still falls through instead of routing through the loop.
    fn compile_dispatch(
        &mut self,
        branch: Branching,
        scopes: &[BlockId],
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        self.enter()?;
        self.stats.dispatch_fallbacks += 1;
        let sel = self.fresh_tmp();
        let label = self.fresh_label();
        let exit = self.fresh_exit();
        let depth = stack.len();
        // Case 0 is the dispatch itself, the scopes are cases 1..n.
        for (i, &x) in scopes.iter().enumerate() {
            stack.push(Scope {
                target: x,
                label,
                kind: SKind::Dispatch {
                    sel,
                    case: i as u32 + 1,
                },
            });
        }
        if let Ft::Block(join) = ft {
            stack.push(Scope {
                target: join,
                label,
                kind: SKind::ExitLoop(exit),
            });
        }

        // Emitted in reverse so each case falls through to the one before it,
        // and the first scope falls through to the join.
        let ordered: Vec<(u32, BlockId)> = scopes
            .iter()
            .enumerate()
            .map(|(i, &x)| (i as u32 + 1, x))
            .rev()
            .collect();
        let dispatch_ft = match ordered.first() {
            Some(&(_, x)) => Ft::Block(x),
            None => ft,
        };
        let (_, dispatch_code) = self.compile_branch_or_test(branch, stack, dispatch_ft)?;
        let mut cases = vec![(0, Region::seq(dispatch_code))];
        let mut last_never = true;
        for (i, &(case, x)) in ordered.iter().enumerate() {
            let case_ft = match ordered.get(i + 1) {
                Some(&(_, next)) => Ft::Block(next),
                None => ft,
            };
            let (never, code) = self.compile_block(x, stack, case_ft)?;
            if i + 1 == ordered.len() {
                last_never = never;
            }
            cases.push((case, Region::seq(code)));
        }
        stack.truncate(depth);
        self.leave();

        let never = !self.exit_used[exit as usize] && last_never;
        Ok((
            never,
            vec![
                Region::SetSel(sel, 0),
                Region::Dispatch {
                    sel,
                    label: Some(label),
                    cases,
                    trailing_break: !never,
                },
            ],
        ))
    }

    /// Emits a jump to a block: either a `break`, a `continue`, or the block
    /// itself when nothing else has claimed it.
    fn compile_branch(
        &mut self,
        target: BlockId,
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        if ft == Ft::Block(target) {
            return Ok((false, Vec::new()));
        }
        let scope = stack.iter().rev().find(|s| s.target == target).copied();
        let Some(scope) = scope else {
            return self.compile_block(target, stack, ft);
        };
        let code = match scope.kind {
            SKind::Loop => {
                let l = self.label_for(Br::Continue, target, scope.label, stack);
                vec![Region::Continue(l)]
            }
            SKind::ExitLoop(exit) | SKind::ExitSwitch(exit) => {
                self.exit_used[exit as usize] = true;
                let l = self.label_for(Br::Break, target, scope.label, stack);
                vec![Region::Break(l)]
            }
            SKind::Forward => {
                self.mark_used(scope.label);
                vec![Region::Break(Some(scope.label))]
            }
            SKind::Dispatch { sel, case } => {
                let l = self.label_for(Br::Continue, target, scope.label, stack);
                vec![Region::SetSel(sel, case), Region::Continue(l)]
            }
        };
        Ok((true, code))
    }

    /// The label a jump has to name, or `None` when leaving the innermost
    /// enclosing construct already lands on the target.
    ///
    /// An unlabeled jump lands on the innermost construct that captures it, so
    /// the label can be dropped exactly when that construct is the target.
    /// Loops capture both forms, switches capture only `break`, labeled blocks
    /// capture neither.
    fn label_for(&mut self, br: Br, target: BlockId, l: Label, stack: &[Scope]) -> Option<Label> {
        let mut can_skip = false;
        for s in stack.iter().rev() {
            match s.kind {
                SKind::Forward => continue,
                SKind::ExitSwitch(_) if br == Br::Continue => continue,
                _ => {
                    if s.label != l {
                        break;
                    }
                    if s.target == target {
                        can_skip = true;
                        break;
                    }
                }
            }
        }
        if can_skip {
            None
        } else {
            self.mark_used(l);
            Some(l)
        }
    }

    fn mark_used(&mut self, l: Label) {
        if let Some(u) = self.label_used.get_mut(l.0 as usize) {
            *u = true;
        }
    }

    fn compile_branch_or_test(
        &mut self,
        branch: Branching,
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        match branch {
            Branching::Goto(b) => self.compile_branch(b, stack, ft),
            Branching::Return => Ok((true, vec![Region::Return])),
            Branching::Unreachable => Ok((true, vec![Region::Unreachable])),
            Branching::Throw => Ok((true, vec![Region::Throw])),
            Branching::Test(pc, dtree) => {
                // The scrutinee is evaluated once: inline when a single test
                // reads it, bound to a temporary otherwise. A tree that tests
                // nothing still evaluates it, for its effects.
                let mut prefix = Vec::new();
                let mut scrut = match dtree.nbcomp() {
                    0 => {
                        let tmp = self.fresh_tmp();
                        prefix.push(Region::LetTmp { tmp, value: pc });
                        None
                    }
                    1 => Some(Scrut::Payload(pc)),
                    _ => {
                        let tmp = self.fresh_tmp();
                        prefix.push(Region::LetTmp { tmp, value: pc });
                        Some(Scrut::Tmp(tmp))
                    }
                };
                let (never, mut code) = self.compile_dtree(&dtree, &mut scrut, stack, ft)?;
                prefix.append(&mut code);
                Ok((never, prefix))
            }
        }
    }

    fn compile_dtree(
        &mut self,
        dtree: &DTree,
        scrut: &mut Option<Scrut<BlockId>>,
        stack: &mut Vec<Scope>,
        ft: Ft,
    ) -> Result<(bool, Vec<Skeleton>), StructureError> {
        match dtree {
            DTree::Branch(b) => self.compile_branch(*b, stack, ft),
            DTree::If(test, a, b) => {
                self.enter()?;
                let (never1, then_) = self.compile_dtree(a, scrut, stack, ft)?;
                let (never2, else_) = self.compile_dtree(b, scrut, stack, ft)?;
                self.leave();
                let scrut = self.take_scrut(scrut)?;
                Ok((
                    never1 && never2,
                    vec![Region::If {
                        scrut,
                        test: *test,
                        then_: Box::new(Region::seq(then_)),
                        else_: Box::new(Region::seq(else_)),
                    }],
                ))
            }
            DTree::Switch { arms, default } => {
                self.enter()?;
                let label = self.fresh_label();
                let exit = self.fresh_exit();
                let depth = stack.len();
                if let Ft::Block(join) = ft {
                    stack.push(Scope {
                        target: join,
                        label,
                        kind: SKind::ExitSwitch(exit),
                    });
                }
                // The default is emitted last, so it is compiled first.
                let (mut all_never, default) = self
                    .compile_branch(*default, stack, ft)
                    .map(|(n, c)| (n, Region::seq(c)))?;
                let mut cases: Vec<(Vec<u128>, Skeleton)> = Vec::with_capacity(arms.len());
                for (values, target) in arms.iter().rev() {
                    let (never, mut code) = self.compile_branch(*target, stack, ft)?;
                    if !never {
                        // Falling off an arm would run the next one.
                        code.push(Region::Break(None));
                        all_never = false;
                    }
                    cases.push((values.clone(), Region::seq(code)));
                }
                cases.reverse();
                stack.truncate(depth);
                self.leave();
                let never = !self.exit_used[exit as usize] && all_never;
                let scrut = self.take_scrut(scrut)?;
                Ok((
                    never,
                    vec![Region::Switch {
                        scrut,
                        arms: cases,
                        default: Box::new(default),
                        label: Some(label),
                    }],
                ))
            }
        }
    }

    fn take_scrut(
        &mut self,
        scrut: &mut Option<Scrut<BlockId>>,
    ) -> Result<Scrut<BlockId>, StructureError> {
        match scrut {
            Some(Scrut::Tmp(t)) => Ok(Scrut::Tmp(*t)),
            other => other
                .take()
                .ok_or(StructureError::Internal("scrutinee read more than once")),
        }
    }

    fn enter(&mut self) -> Result<(), StructureError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(StructureError::TooDeep);
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Drops the labels no jump names, which is where the guarantee that every
    /// label in the output is named by some jump comes from.
    ///
    /// Emission attaches a label to every construct that could carry one and
    /// records the ones jumps ask for; this pass is the single place that
    /// decides which survive.
    fn drop_unused_labels(&self, r: Skeleton) -> Skeleton {
        let keep = |l: Option<Label>| l.filter(|l| self.used(*l).is_some());
        match r {
            Region::Seq(v) => {
                Region::Seq(v.into_iter().map(|r| self.drop_unused_labels(r)).collect())
            }
            Region::Labeled { label, body } => {
                let body = self.drop_unused_labels(*body);
                if self.used(label).is_some() {
                    Region::Labeled {
                        label,
                        body: Box::new(body),
                    }
                } else {
                    body
                }
            }
            Region::Loop { label, body } => Region::Loop {
                label: keep(label),
                body: Box::new(self.drop_unused_labels(*body)),
            },
            Region::If {
                scrut,
                test,
                then_,
                else_,
            } => Region::If {
                scrut,
                test,
                then_: Box::new(self.drop_unused_labels(*then_)),
                else_: Box::new(self.drop_unused_labels(*else_)),
            },
            Region::Switch {
                scrut,
                arms,
                default,
                label,
            } => Region::Switch {
                scrut,
                arms: arms
                    .into_iter()
                    .map(|(v, r)| (v, self.drop_unused_labels(r)))
                    .collect(),
                default: Box::new(self.drop_unused_labels(*default)),
                label: keep(label),
            },
            Region::Dispatch {
                sel,
                label,
                cases,
                trailing_break,
            } => Region::Dispatch {
                sel,
                label: keep(label),
                cases: cases
                    .into_iter()
                    .map(|(c, r)| (c, self.drop_unused_labels(r)))
                    .collect(),
                trailing_break,
            },
            r => r,
        }
    }
}

/// A terminator reduced to how it branches.
enum Branching {
    Goto(BlockId),
    /// A decision tree over the scrutinee of the named block's terminator.
    Test(BlockId, DTree),
    Return,
    Unreachable,
    Throw,
}

impl Branching {
    fn nbbranch(&self, target: BlockId) -> u32 {
        match self {
            Branching::Test(_, d) => d.nbbranch(target),
            _ => 0,
        }
    }
}
