//! Graphs at the edges of what the input allows.

mod support;

use expect_test::expect;
use structurizer::{structure, Block, BlockId, Cfg, Options, StructureError, Term};
use support::*;

#[test]
fn single_block() {
    let out = run(vec![ret()]);
    expect![[r#"
        b0;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn empty_statements() {
    let out = run_cfg(cfg_with(vec![("", goto(1)), ("", ret())]));
    expect![[r#"
        ;
        ;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn throwing_and_unreachable_terminators() {
    let out = run(vec![cond("c0", 1, 2), throw(), unreachable()]);
    expect![[r#"
        b0;
        if (c0) {
          b1;
          throw;
        } else {
          b2;
          unreachable;
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn unreachable_blocks_are_dropped() {
    // Blocks 2 and 3 cannot be reached, and 3 even branches to a block that
    // does not exist in the reachable part.
    let out = run(vec![goto(1), ret(), goto(3), goto(2)]);
    expect![[r#"
        b0;
        b1;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn both_arms_of_a_branch_on_one_block() {
    let out = run(vec![cond("c0", 1, 1), ret()]);
    expect![[r#"
        b0;
        if (c0) {
        }
        b1;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn a_switch_with_no_cases() {
    let out = run(vec![switch("c0", &[], 1), ret()]);
    expect![[r#"
        b0;
        let $t0 = c0;
        b1;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn repeated_case_values_keep_the_first() {
    let out = run(vec![switch("c0", &[(1, 1), (1, 2)], 2), ret(), ret()]);
    expect![[r#"
        b0;
        if (c0 == 1) {
          b1;
          return;
        } else {
          b2;
          return;
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn an_unwind_edge_is_not_an_edge() {
    // Block 1 is only named by an unwind field, so it is not reachable.
    let cfg = Cfg {
        entry: BlockId(0),
        blocks: vec![
            Block { stmts: "b0", term: Term::Return, unwind: Some(BlockId(1)) },
            Block { stmts: "b1", term: Term::Throw, unwind: None },
        ],
    };
    let out = run_cfg(cfg);
    expect![[r#"
        b0;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn a_branch_to_nowhere_is_an_error() {
    let cfg = cfg(vec![goto(7), ret()]);
    let err = structure(cfg, &Options::default()).unwrap_err();
    assert_eq!(
        err,
        StructureError::BlockOutOfRange { from: Some(BlockId(0)), target: BlockId(7) }
    );
    assert_eq!(err.to_string(), "block 0 branches to unknown block 7");
}

#[test]
fn an_empty_body_is_an_error() {
    let cfg: Cfg<S, S> = Cfg { entry: BlockId(0), blocks: Vec::new() };
    assert_eq!(
        structure(cfg, &Options::default()).unwrap_err(),
        StructureError::EmptyCfg
    );
}

#[test]
fn an_entry_outside_the_body_is_an_error() {
    let mut cfg = cfg(vec![ret()]);
    cfg.entry = BlockId(4);
    assert_eq!(
        structure(cfg, &Options::default()).unwrap_err(),
        StructureError::EmptyCfg
    );
}

/// `n` two armed branches in a row, each joining before the next.
fn diamond_chain(n: u32) -> Cfg<S, S> {
    let mut blocks = Vec::new();
    for i in 0..n {
        let base = i * 3;
        blocks.push(Block {
            stmts: "s",
            term: Term::Cond {
                on: "c",
                then_: BlockId(base + 1),
                else_: BlockId(base + 2),
            },
            unwind: None,
        });
        blocks.push(Block { stmts: "s", term: Term::Goto(BlockId(base + 3)), unwind: None });
        blocks.push(Block { stmts: "s", term: Term::Goto(BlockId(base + 3)), unwind: None });
    }
    blocks.push(Block { stmts: "s", term: Term::Return, unwind: None });
    Cfg { entry: BlockId(0), blocks }
}

#[test]
fn deep_nesting_falls_back_instead_of_overflowing() {
    // Well inside the nesting limit: one region tree, no fallback.
    let out = structure(diamond_chain(200), &Options::default()).unwrap();
    assert_eq!(out.stats.dispatch_fallbacks, 0);

    // Past it: the whole body becomes one dispatch loop, whatever its size.
    for n in [400, 1600, 3200] {
        let out = structure(diamond_chain(n), &Options::default()).unwrap();
        assert_eq!(out.stats.dispatch_fallbacks, 1, "{n} branches");
        assert!(!out.stats.irreducible);
    }
}
