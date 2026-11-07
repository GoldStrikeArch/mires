//! Where control joins, and what happens when too many joins are siblings.

mod support;

use expect_test::expect;
use structurizer::{Options, Term};
use support::*;

#[test]
fn single_diamond() {
    // Both arms reach block 3, which needs no label: it is simply emitted
    // after the if.
    let out = run(vec![cond("c0", 1, 2), goto(3), goto(3), ret()]);
    expect![[r#"
        b0;
        if (c0) {
          b1;
        } else {
          b2;
        }
        b3;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn two_sequential_diamonds() {
    let out = run(vec![
        cond("c0", 1, 2),
        goto(3),
        goto(3),
        cond("c3", 4, 5),
        goto(6),
        goto(6),
        ret(),
    ]);
    expect![[r#"
        b0;
        if (c0) {
          b1;
        } else {
          b2;
        }
        b3;
        if (c3) {
          b4;
        } else {
          b5;
        }
        b6;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn triple_merge() {
    let out = run(vec![
        switch("c0", &[(0, 1), (1, 2), (2, 3)], 4),
        goto(4),
        goto(4),
        goto(4),
        ret(),
    ]);
    expect![[r#"
        b0;
        switch (c0) {
          case 0:
            b1;
            break;
          case 1:
            b2;
            break;
          case 2:
            b3;
            break;
          default:
        }
        b4;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn merge_from_inside_and_outside_a_loop() {
    // Block 4 is reached from before the loop and from inside it. Leaving the
    // loop lands on it, so the jump out needs no label.
    let out = run(vec![
        cond("c0", 1, 4),
        goto(2),
        cond("c2", 3, 4),
        goto(2),
        ret(),
    ]);
    expect![[r#"
        b0;
        if (c0) {
          b1;
          loop {
            b2;
            if (c2) {
              b3;
            } else {
              break;
            }
          }
        }
        b4;
        return;
    "#]]
    .assert_eq(&out);
}

/// A block dominating `n` sibling join points, each reached from the switch
/// and from the previous one.
fn siblings(n: u32) -> Vec<Term<S>> {
    let cases: Vec<(u128, u32)> = (0..n).map(|i| (i as u128, i + 1)).collect();
    let mut blocks = vec![switch("c0", &cases, 1)];
    for i in 1..=n {
        blocks.push(if i == n { ret() } else { goto(i + 1) });
    }
    blocks
}

#[test]
fn nine_siblings_nest() {
    let out = run(siblings(9));
    expect![[r#"
        // labels: L0 L1 L2 L3 L4 L5 L6
        b0;
        L0: {
          L1: {
            L2: {
              L3: {
                L4: {
                  L5: {
                    L6: {
                      switch (c0) {
                        case 1:
                          break;
                        case 2:
                          break L6;
                        case 3:
                          break L5;
                        case 4:
                          break L4;
                        case 5:
                          break L3;
                        case 6:
                          break L2;
                        case 7:
                          break L1;
                        case 8:
                          break L0;
                        default:
                          b1;
                      }
                      b2;
                    }
                    b3;
                  }
                  b4;
                }
                b5;
              }
              b6;
            }
            b7;
          }
          b8;
        }
        b9;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn twelve_siblings_dispatch() {
    let out = run(siblings(12));
    expect![[r#"
        // dispatch x1
        b0;
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              switch (c0) {
                case 1:
                  break;
                case 2:
                  $t0 = 10;
                  continue;
                case 3:
                  $t0 = 9;
                  continue;
                case 4:
                  $t0 = 8;
                  continue;
                case 5:
                  $t0 = 7;
                  continue;
                case 6:
                  $t0 = 6;
                  continue;
                case 7:
                  $t0 = 5;
                  continue;
                case 8:
                  $t0 = 4;
                  continue;
                case 9:
                  $t0 = 3;
                  continue;
                case 10:
                  $t0 = 2;
                  continue;
                case 11:
                  $t0 = 1;
                  continue;
                default:
                  b1;
              }
            case 11:
              b2;
            case 10:
              b3;
            case 9:
              b4;
            case 8:
              b5;
            case 7:
              b6;
            case 6:
              b7;
            case 5:
              b8;
            case 4:
              b9;
            case 3:
              b10;
            case 2:
              b11;
            case 1:
              b12;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn sibling_threshold_moves_with_the_option() {
    let opts = Options { merge_node_max: 3, ..Options::default() };
    let out = run_opts(cfg(siblings(5)), &opts);
    expect![[r#"
        // dispatch x1
        b0;
        $t0 = 0;
        loop {
          switch ($t0) {
            case 0:
              switch (c0) {
                case 1:
                  break;
                case 2:
                  $t0 = 3;
                  continue;
                case 3:
                  $t0 = 2;
                  continue;
                case 4:
                  $t0 = 1;
                  continue;
                default:
                  b1;
              }
            case 4:
              b2;
            case 3:
              b3;
            case 2:
              b4;
            case 1:
              b5;
              return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn goto_chain_flattens() {
    // Fifteen blocks in a row: one sequence, no labels, no loops.
    let mut blocks: Vec<Term<S>> = (1..15).map(goto).collect();
    blocks.push(ret());
    let out = run(blocks);
    expect![[r#"
        b0;
        b1;
        b2;
        b3;
        b4;
        b5;
        b6;
        b7;
        b8;
        b9;
        b10;
        b11;
        b12;
        b13;
        b14;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn diamond_with_long_arms() {
    let out = run(vec![
        cond("c0", 1, 4),
        goto(2),
        goto(3),
        goto(7),
        goto(5),
        goto(6),
        goto(7),
        ret(),
    ]);
    expect![[r#"
        b0;
        if (c0) {
          b1;
          b2;
          b3;
        } else {
          b4;
          b5;
          b6;
        }
        b7;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn continue_across_an_inner_loop_is_labeled() {
    // Block 4 jumps to the outer loop header from inside the inner loop, whose
    // own exit is elsewhere. An unlabeled continue would restart the inner
    // loop, so the outer one has to be named.
    let out = run(vec![
        goto(1),
        cond("c1", 2, 9),
        cond("c2", 3, 7),
        cond("c3", 4, 6),
        cond("c4", 1, 5),
        goto(3),
        goto(8),
        goto(8),
        goto(1),
        ret(),
    ]);
    expect![[r#"
        // labels: L0
        b0;
        L0: loop {
          b1;
          if (c1) {
            b2;
            if (c2) {
              loop {
                b3;
                if (c3) {
                  b4;
                  if (c4) {
                    continue L0;
                  } else {
                    b5;
                  }
                } else {
                  b6;
                  break;
                }
              }
            } else {
              b7;
            }
            b8;
          } else {
            b9;
            return;
          }
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn break_across_an_inner_loop_is_labeled() {
    // The same shape, but block 4 leaves the outer loop instead.
    let out = run(vec![
        goto(1),
        cond("c1", 2, 9),
        cond("c2", 3, 7),
        cond("c3", 4, 6),
        cond("c4", 9, 5),
        goto(3),
        goto(8),
        goto(8),
        goto(1),
        ret(),
    ]);
    expect![[r#"
        // labels: L1
        b0;
        loop {
          b1;
          L1: {
            if (c1) {
              b2;
              if (c2) {
                loop {
                  b3;
                  if (c3) {
                    b4;
                    if (c4) {
                      break L1;
                    } else {
                      b5;
                    }
                  } else {
                    b6;
                    break;
                  }
                }
              } else {
                b7;
              }
              b8;
              continue;
            }
          }
          b9;
          return;
        }
    "#]]
    .assert_eq(&out);
}
