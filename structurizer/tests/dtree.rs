//! How the values of a multi way branch are split into tests.

mod support;

use expect_test::expect;
use structurizer::{structure, Options};
use support::*;

/// A switch over `cases` where every target is a block that returns.
fn switch_cfg(cases: &[(u128, u32)], default: u32, targets: u32) -> Vec<structurizer::Term<S>> {
    let mut blocks = vec![switch("c0", cases, default)];
    for _ in 0..targets {
        blocks.push(ret());
    }
    blocks
}

#[test]
fn contiguous_cases_with_one_target_group_up() {
    let cases = [(0, 1), (1, 1), (2, 1), (3, 2), (4, 2)];
    let out = run(switch_cfg(&cases, 3, 3));
    expect![[r#"
        b0;
        switch (c0) {
          case 3, 4:
            b2;
            return;
          case 0, 1, 2:
            b1;
            return;
          default:
            b3;
            return;
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn one_value_is_an_equality() {
    let out = run(switch_cfg(&[(5, 1)], 2, 2));
    expect![[r#"
        b0;
        if (c0 == 5) {
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
fn a_band_of_values_is_a_range() {
    let out = run(switch_cfg(&[(5, 1), (6, 1), (7, 1)], 2, 2));
    expect![[r#"
        b0;
        if (c0 in [5..8)) {
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
fn every_case_on_the_default_is_a_plain_branch() {
    let out = run(switch_cfg(&[(0, 1), (1, 1), (7, 1)], 1, 1));
    expect![[r#"
        b0;
        let $t0 = c0;
        b1;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn the_widest_group_becomes_the_default() {
    // Forty values over four targets, one of which is also the fallback: its
    // ten values need no cases of their own.
    let cases: Vec<(u128, u32)> = (0..40).map(|v| (v as u128, v % 4 + 1)).collect();
    let out = run(switch_cfg(&cases, 1, 4));
    expect![[r#"
        b0;
        switch (c0) {
          case 1, 5, 9, 13, 17, 21, 25, 29, 33, 37:
            b2;
            return;
          case 2, 6, 10, 14, 18, 22, 26, 30, 34, 38:
            b3;
            return;
          case 3, 7, 11, 15, 19, 23, 27, 31, 35, 39:
            b4;
            return;
          default:
            b1;
            return;
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn forty_arms_stay_one_switch() {
    let cases: Vec<(u128, u32)> = (0..40).map(|v| (v as u128, v % 4 + 1)).collect();
    let out = run(switch_cfg(&cases, 5, 5));
    expect![[r#"
        b0;
        switch (c0) {
          case 0, 4, 8, 12, 16, 20, 24, 28, 32, 36:
            b1;
            return;
          case 1, 5, 9, 13, 17, 21, 25, 29, 33, 37:
            b2;
            return;
          case 2, 6, 10, 14, 18, 22, 26, 30, 34, 38:
            b3;
            return;
          case 3, 7, 11, 15, 19, 23, 27, 31, 35, 39:
            b4;
            return;
          default:
            b5;
            return;
        }
    "#]]
    .assert_eq(&out);
}

#[test]
fn two_hundred_arms_are_split() {
    let cases: Vec<(u128, u32)> = (0..200).map(|v| (v as u128, v % 5 + 1)).collect();
    let cfg = cfg(switch_cfg(&cases, 6, 6));
    let out = structure(cfg, &Options::default()).unwrap();
    expect![[r#"
        blocks: 7
        tests: Le Le Le
        switches: 4 with [51, 41, 41, 51] cases
    "#]]
    .assert_eq(&summarize(&out.region));
}

#[test]
fn a_covered_range_splits_by_comparison() {
    // Eight pairs of values with a low switch limit, so the tree keeps
    // splitting until it reaches a range it fully covers and can compare
    // against directly.
    let cases: Vec<(u128, u32)> = (0..16).map(|v| (v as u128, v / 2 + 1)).collect();
    let opts = Options { switch_max_case: 4, ..Options::default() };
    let out = run_opts(cfg(switch_cfg(&cases, 9, 9)), &opts);
    expect![[r#"
        b0;
        let $t0 = c0;
        if (8 <= $t0) {
          if (12 <= $t0) {
            if (14 <= $t0) {
              if ($t0 in [14..16)) {
                b8;
                return;
              }
            } else {
              b7;
              return;
            }
          } else {
            if (9 < $t0) {
              b6;
              return;
            } else {
              b5;
              return;
            }
          }
        } else {
          if (4 <= $t0) {
            if (5 < $t0) {
              b4;
              return;
            } else {
              b3;
              return;
            }
          } else {
            if (2 <= $t0) {
              b2;
              return;
            } else {
              if ($t0 in [0..2)) {
                b1;
                return;
              }
            }
          }
        }
        b9;
        return;
    "#]]
    .assert_eq(&out);
}

#[test]
fn a_scrutinee_read_twice_is_bound_once() {
    // A tree of comparisons over the same value: it is evaluated into a
    // temporary first, so it is read once however many tests there are.
    let cases = [(0, 1), (5, 2), (9, 3)];
    let opts = Options { switch_max_case: 2, ..Options::default() };
    let out = run_opts(cfg(switch_cfg(&cases, 4, 4)), &opts);
    expect![[r#"
        b0;
        let $t0 = c0;
        if (9 <= $t0) {
          if ($t0 == 9) {
            b3;
            return;
          }
        } else {
          if (5 <= $t0) {
            if ($t0 == 5) {
              b2;
              return;
            }
          } else {
            if ($t0 == 0) {
              b1;
              return;
            }
          }
        }
        b4;
        return;
    "#]]
    .assert_eq(&out);
}
