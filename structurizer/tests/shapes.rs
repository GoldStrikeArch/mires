//! Every shape of control flow crossed with every way of leaving it.

mod support;

use expect_test::expect;
use structurizer::Term;
use support::*;

/// What the hot block of a shape does.
#[derive(Clone, Copy)]
enum Exit {
    BreakInner,
    BreakOuter,
    ContinueInner,
    ContinueOuter,
    Return,
    Fallthrough,
}

const EXITS: [(&str, Exit); 6] = [
    ("break inner", Exit::BreakInner),
    ("break outer", Exit::BreakOuter),
    ("continue inner", Exit::ContinueInner),
    ("continue outer", Exit::ContinueOuter),
    ("return", Exit::Return),
    ("fallthrough", Exit::Fallthrough),
];

/// Where each way out of a shape leads. `None` means the shape has no such
/// construct to leave.
#[derive(Clone, Copy, Default)]
struct Ways {
    inner_head: Option<u32>,
    inner_exit: Option<u32>,
    outer_head: Option<u32>,
    outer_exit: Option<u32>,
    next: u32,
}

impl Ways {
    fn term(&self, e: Exit) -> Option<Term<S>> {
        match e {
            Exit::BreakInner => self.inner_exit.map(goto),
            Exit::BreakOuter => self.outer_exit.map(goto),
            Exit::ContinueInner => self.inner_head.map(goto),
            Exit::ContinueOuter => self.outer_head.map(goto),
            Exit::Return => Some(ret()),
            Exit::Fallthrough => Some(goto(self.next)),
        }
    }
}

/// Renders one shape once per way out.
fn matrix(blocks: Vec<Term<S>>, hot: usize, ways: Ways) -> String {
    let mut out = String::new();
    for (name, exit) in EXITS {
        let body = match ways.term(exit) {
            None => "n/a\n".to_string(),
            Some(t) => {
                let mut blocks: Vec<Term<S>> = blocks.iter().map(clone_term).collect();
                blocks[hot] = t;
                run(blocks)
            }
        };
        out.push_str(&section(name, body));
    }
    out
}

fn clone_term(t: &Term<S>) -> Term<S> {
    match t {
        Term::Goto(b) => Term::Goto(*b),
        Term::Cond { on, then_, else_ } => Term::Cond { on, then_: *then_, else_: *else_ },
        Term::Switch { on, cases, default } => {
            Term::Switch { on, cases: cases.clone(), default: *default }
        }
        Term::Return => Term::Return,
        Term::Unreachable => Term::Unreachable,
        Term::Throw => Term::Throw,
    }
}

#[test]
fn if_shape() {
    // 0: if (c0) { 1 } ; 2
    let blocks = vec![cond("c0", 1, 2), goto(2), ret()];
    let ways = Ways { next: 2, ..Ways::default() };
    expect![[r#"
        == break inner ==
        n/a
        == break outer ==
        n/a
        == continue inner ==
        n/a
        == continue outer ==
        n/a
        == return ==
        b0;
        if (c0) {
          b1;
          return;
        } else {
          b2;
          return;
        }
        == fallthrough ==
        b0;
        if (c0) {
          b1;
        }
        b2;
        return;
    "#]]
    .assert_eq(&matrix(blocks, 1, ways));
}

#[test]
fn if_else_shape() {
    // 0: if (c0) { 1 } else { 2 } ; 3
    let blocks = vec![cond("c0", 1, 2), goto(3), goto(3), ret()];
    let ways = Ways { next: 3, ..Ways::default() };
    expect![[r#"
        == break inner ==
        n/a
        == break outer ==
        n/a
        == continue inner ==
        n/a
        == continue outer ==
        n/a
        == return ==
        b0;
        if (c0) {
          b1;
          return;
        } else {
          b2;
          b3;
          return;
        }
        == fallthrough ==
        b0;
        if (c0) {
          b1;
        } else {
          b2;
        }
        b3;
        return;
    "#]]
    .assert_eq(&matrix(blocks, 1, ways));
}

#[test]
fn while_shape() {
    // 1: while (c1) { 2; 3 } ; 4
    let blocks = vec![goto(1), cond("c1", 2, 4), goto(3), goto(1), ret()];
    let ways = Ways { inner_head: Some(1), inner_exit: Some(4), next: 3, ..Ways::default() };
    expect![[r#"
        == break inner ==
        b0;
        b1;
        if (c1) {
          b2;
        }
        b4;
        return;
        == break outer ==
        n/a
        == continue inner ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
          } else {
            b4;
            return;
          }
        }
        == continue outer ==
        n/a
        == return ==
        b0;
        b1;
        if (c1) {
          b2;
          return;
        } else {
          b4;
          return;
        }
        == fallthrough ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            b3;
          } else {
            b4;
            return;
          }
        }
    "#]]
    .assert_eq(&matrix(blocks, 2, ways));
}

#[test]
fn loop_shape() {
    // 1: loop { 1; if (c2) continue; else break } ; 3
    let blocks = vec![goto(1), goto(2), cond("c2", 1, 3), ret()];
    let ways = Ways { inner_head: Some(1), inner_exit: Some(3), next: 2, ..Ways::default() };
    expect![[r#"
        == break inner ==
        b0;
        b1;
        b3;
        return;
        == break outer ==
        n/a
        == continue inner ==
        b0;
        loop {
          b1;
        }
        == continue outer ==
        n/a
        == return ==
        b0;
        b1;
        return;
        == fallthrough ==
        b0;
        loop {
          b1;
          b2;
          if (c2) {
          } else {
            b3;
            return;
          }
        }
    "#]]
    .assert_eq(&matrix(blocks, 1, ways));
}

#[test]
fn if_in_if_shape() {
    // 0: if (c0) { if (c1) { 2 } else { 3 } } ; 4
    let blocks = vec![cond("c0", 1, 4), cond("c1", 2, 3), goto(4), goto(4), ret()];
    let ways = Ways { next: 4, ..Ways::default() };
    expect![[r#"
        == break inner ==
        n/a
        == break outer ==
        n/a
        == continue inner ==
        n/a
        == continue outer ==
        n/a
        == return ==
        b0;
        if (c0) {
          b1;
          if (c1) {
            b2;
            return;
          } else {
            b3;
          }
        }
        b4;
        return;
        == fallthrough ==
        b0;
        if (c0) {
          b1;
          if (c1) {
            b2;
          } else {
            b3;
          }
        }
        b4;
        return;
    "#]]
    .assert_eq(&matrix(blocks, 2, ways));
}

#[test]
fn loop_in_loop_shape() {
    // 1: while (c1) { 3: while (c3) { 4; 7 } ; 5 } ; 6
    let blocks = vec![
        goto(1),
        cond("c1", 2, 6),
        goto(3),
        cond("c3", 4, 5),
        goto(7),
        goto(1),
        ret(),
        goto(3),
    ];
    let ways = Ways {
        inner_head: Some(3),
        inner_exit: Some(5),
        outer_head: Some(1),
        outer_exit: Some(6),
        next: 7,
    };
    expect![[r#"
        == break inner ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            b3;
            if (c3) {
              b4;
            }
            b5;
          } else {
            b6;
            return;
          }
        }
        == break outer ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            b3;
            if (c3) {
              b4;
            } else {
              b5;
              continue;
            }
          }
          b6;
          return;
        }
        == continue inner ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            loop {
              b3;
              if (c3) {
                b4;
              } else {
                b5;
                break;
              }
            }
          } else {
            b6;
            return;
          }
        }
        == continue outer ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            b3;
            if (c3) {
              b4;
            } else {
              b5;
            }
          } else {
            b6;
            return;
          }
        }
        == return ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            b3;
            if (c3) {
              b4;
              return;
            } else {
              b5;
            }
          } else {
            b6;
            return;
          }
        }
        == fallthrough ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            loop {
              b3;
              if (c3) {
                b4;
                b7;
              } else {
                b5;
                break;
              }
            }
          } else {
            b6;
            return;
          }
        }
    "#]]
    .assert_eq(&matrix(blocks, 4, ways));
}

#[test]
fn loop_in_if_shape() {
    // 0: if (c0) { 2: while (c2) { 3; 6 } ; 4 } ; 5
    let blocks = vec![
        cond("c0", 1, 5),
        goto(2),
        cond("c2", 3, 4),
        goto(6),
        goto(5),
        ret(),
        goto(2),
    ];
    let ways = Ways { inner_head: Some(2), inner_exit: Some(4), next: 6, ..Ways::default() };
    expect![[r#"
        == break inner ==
        b0;
        if (c0) {
          b1;
          b2;
          if (c2) {
            b3;
          }
          b4;
        }
        b5;
        return;
        == break outer ==
        n/a
        == continue inner ==
        b0;
        if (c0) {
          b1;
          loop {
            b2;
            if (c2) {
              b3;
            } else {
              b4;
              break;
            }
          }
        }
        b5;
        return;
        == continue outer ==
        n/a
        == return ==
        b0;
        if (c0) {
          b1;
          b2;
          if (c2) {
            b3;
            return;
          } else {
            b4;
          }
        }
        b5;
        return;
        == fallthrough ==
        b0;
        if (c0) {
          b1;
          loop {
            b2;
            if (c2) {
              b3;
              b6;
            } else {
              b4;
              break;
            }
          }
        }
        b5;
        return;
    "#]]
    .assert_eq(&matrix(blocks, 3, ways));
}

#[test]
fn if_in_loop_shape() {
    // 1: while (c1) { if (c2) { 3 } else { 4 } ; 5 } ; 6
    let blocks = vec![
        goto(1),
        cond("c1", 2, 6),
        cond("c2", 3, 4),
        goto(5),
        goto(5),
        goto(1),
        ret(),
    ];
    let ways = Ways { inner_head: Some(1), inner_exit: Some(6), next: 5, ..Ways::default() };
    expect![[r#"
        == break inner ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            if (c2) {
              b3;
            } else {
              b4;
              b5;
              continue;
            }
          }
          b6;
          return;
        }
        == break outer ==
        n/a
        == continue inner ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            if (c2) {
              b3;
            } else {
              b4;
              b5;
            }
          } else {
            b6;
            return;
          }
        }
        == continue outer ==
        n/a
        == return ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            if (c2) {
              b3;
              return;
            } else {
              b4;
              b5;
            }
          } else {
            b6;
            return;
          }
        }
        == fallthrough ==
        b0;
        loop {
          b1;
          if (c1) {
            b2;
            if (c2) {
              b3;
            } else {
              b4;
            }
            b5;
          } else {
            b6;
            return;
          }
        }
    "#]]
    .assert_eq(&matrix(blocks, 3, ways));
}
