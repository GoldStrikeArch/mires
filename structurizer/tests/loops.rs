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
    let opts = Options { small_limit: 100, ..Options::default() };
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
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              b0;
              if (c0) {
                $t0 = 1;
                continue;
              } else {
                $t0 = 2;
                continue;
              }
            case 1:
              b1;
              if (c1) {
                $t0 = 2;
                continue;
              } else {
                $t0 = 3;
                continue;
              }
            case 2:
              b2;
              if (c2) {
                $t0 = 1;
                continue;
              } else {
                $t0 = 3;
                continue;
              }
            case 3:
              b3;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn forced_dispatch() {
    let opts = Options { force_dispatch: true, ..Options::default() };
    let out = run_opts(cfg(vec![cond("c0", 1, 2), goto(2), ret()]), &opts);
    expect![[r#"
        // dispatch x1
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              b0;
              if (c0) {
                $t0 = 1;
                continue;
              } else {
                $t0 = 2;
                continue;
              }
            case 1:
              b1;
              $t0 = 2;
              continue;
            case 2:
              b2;
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
