//! Loop continuations and control flow that is not reducible.

mod support;

use expect_test::expect;
use structurizer::{Options, Term};
use support::*;

/// Twenty one statements: one over the default `small_limit`.
const BIG: &str = ";;;;;;;;;;;;;;;;;;;;;";
/// One statement.
const SMALL: &str = ";";

/// A loop whose exit continuation carries the given statements.
///
///   0 -> 1; 1: while (c1) { 2 }; 3; 4
fn loop_with_continuation(cont: &'static str) -> Vec<(&'static str, Term<S>)> {
    vec![
        ("b0", goto(1)),
        ("b1", cond("c1", 2, 3)),
        ("b2", goto(1)),
        (cont, goto(4)),
        ("b4", ret()),
    ]
}

#[test]
fn big_continuation_moves_out_of_the_loop() {
    let out = run_cfg(cfg_with(loop_with_continuation(BIG)));
    expect![[r#"
        b0;
        loop {
          b1;
          if (c1) {
            b2;
          } else {
            break;
          }
        }
        ;;;;;;;;;;;;;;;;;;;;;;
        b4;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn small_continuation_stays_in_the_loop() {
    let out = run_cfg(cfg_with(loop_with_continuation(SMALL)));
    expect![[r#"
        b0;
        loop {
          b1;
          if (c1) {
            b2;
          } else {
            ;;
            b4;
            return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn the_small_limit_decides() {
    // The same graph as the big case, with a limit that admits it.
    let opts = Options {
        small_limit: 100,
        ..Options::default()
    };
    let out = run_opts(cfg_with(loop_with_continuation(BIG)), &opts);
    expect![[r#"
        b0;
        loop {
          b1;
          if (c1) {
            b2;
          } else {
            ;;;;;;;;;;;;;;;;;;;;;;
            b4;
            return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn irreducible_two_entry_loop() {
    // Blocks 1 and 2 form a cycle entered at both of them, which no
    // arrangement of loops can express.
    let out = run(vec![
        cond("c0", 1, 2),
        cond("c1", 2, 3),
        cond("c2", 1, 3),
        ret(),
    ]);
    expect![[r#"
        // irreducible
        // dispatch x1
        b0;
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              if (c0) {
              } else {
                $t0 = 2;
                continue;
              }
            case 3:
              b1;
              if (c1) {
              } else {
                $t0 = 1;
                continue;
              }
            case 2:
              b2;
              if (c2) {
                $t0 = 3;
                continue;
              }
            case 1:
              b3;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn forced_dispatch() {
    let opts = Options {
        force_dispatch: true,
        ..Options::default()
    };
    let out = run_opts(cfg(vec![cond("c0", 1, 2), goto(2), ret()]), &opts);
    expect![[r#"
        // dispatch x1 (whole body)
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              b0;
              if (c0) {
              } else {
                $t0 = 2;
                continue;
              }
            case 1:
              b1;
            case 2:
              b2;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn forced_dispatch_absorbs_goto_chains() {
    // A straight line keeps one case: each block's one predecessor hands
    // control over unconditionally, so the chain concatenates.
    let opts = Options {
        force_dispatch: true,
        ..Options::default()
    };
    let out = run_opts(cfg(vec![goto(1), goto(2), ret()]), &opts);
    expect![[r#"
        // dispatch x1 (whole body)
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              b0;
              b1;
              b2;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn two_entry_loop_keeps_the_rest_nested() {
    // Blocks 2 and 3 form a cycle entered at both, through straight line
    // stretches 1 and 4. Only the cycle pays for a dispatch: the entry
    // statements stay outside it and the stretches inline into its cases.
    let out = run(vec![
        cond("c0", 1, 4),
        goto(2),
        cond("c2", 3, 5),
        cond("c3", 2, 5),
        goto(3),
        ret(),
    ]);
    expect![[r#"
        // irreducible
        // dispatch x1
        b0;
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              if (c0) {
                b1;
              } else {
                b4;
                $t0 = 2;
                continue;
              }
            case 3:
              b2;
              if (c2) {
              } else {
                $t0 = 1;
                continue;
              }
            case 2:
              b3;
              if (c3) {
                $t0 = 3;
                continue;
              }
            case 1:
              b5;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn dominated_loop_inside_a_dispatch_member() {
    // Blocks 2 and 4 head a two entry cycle, and block 5, dominated by 2,
    // loops on itself. The self loop stays a loop inside 2's case; only the
    // retreating jump between the entries goes through the selector.
    let out = run(vec![
        cond("c0", 1, 3),
        goto(2),
        goto(5),
        goto(4),
        cond("c4", 2, 6),
        cond("c5", 5, 4),
        ret(),
    ]);
    expect![[r#"
        // irreducible
        // dispatch x1
        b0;
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              if (c0) {
                b1;
              } else {
                b3;
                $t0 = 1;
                continue;
              }
            case 2:
              b2;
              loop {
                b5;
                if (c5) {
                } else {
                  break;
                }
              }
            case 1:
              b4;
              if (c4) {
                $t0 = 2;
                continue;
              } else {
                b6;
                return;
              }
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn three_entry_cycle() {
    let out = run(vec![
        switch("s0", &[(0, 1), (1, 2), (2, 3)], 4),
        cond("c1", 2, 4),
        cond("c2", 3, 4),
        cond("c3", 1, 4),
        ret(),
    ]);
    expect![[r#"
        // irreducible
        // dispatch x1
        b0;
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              switch (s0) {
                case 0:
                  break;
                case 1:
                  $t0 = 3;
                  continue;
                case 2:
                  $t0 = 2;
                  continue;
                default:
                  $t0 = 1;
                  continue;
              }
            case 4:
              b1;
              if (c1) {
              } else {
                $t0 = 1;
                continue;
              }
            case 3:
              b2;
              if (c2) {
              } else {
                $t0 = 1;
                continue;
              }
            case 2:
              b3;
              if (c3) {
                $t0 = 4;
                continue;
              }
            case 1:
              b4;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn self_loop() {
    let out = run(vec![goto(1), cond("c1", 1, 2), ret()]);
    expect![[r#"
        b0;
        loop {
          b1;
          if (c1) {
          } else {
            b2;
            return;
          }
        }
    "#]]
    .assert_eq(&out);
}
