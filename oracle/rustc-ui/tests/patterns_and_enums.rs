//! `rustc/tests/ui`-style tests for pattern matching, guards, bindings, enums, and niches.

use std::num::{NonZeroI32, NonZeroU8, NonZeroU32};

#[derive(Debug, PartialEq, Eq)]
enum Expr {
    Lit(i64),
    Neg(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Pair { left: i32, right: i32 },
}

fn eval(e: &Expr) -> i64 {
    match e {
        Expr::Lit(n) => *n,
        Expr::Neg(inner) => -eval(inner),
        Expr::Add(a, b) => eval(a) + eval(b),
        Expr::Pair { left, right } => (*left as i64) * 100 + (*right as i64),
    }
}

#[test]
fn ui_recursive_enum_eval() {
    let e = Expr::Add(
        Box::new(Expr::Neg(Box::new(Expr::Lit(12)))),
        Box::new(Expr::Pair { left: 3, right: 45 }),
    );
    assert_eq!(eval(&e), 333);
}

#[test]
fn ui_ref_and_ref_mut_pattern_bindings() {
    let mut data = (String::from("hello"), vec![1, 2, 3]);
    match data {
        (ref mut s, ref mut v) if v.len() == 3 => {
            s.push_str(", world");
            v.push(4);
        }
        _ => unreachable!(),
    }
    assert_eq!(data.0, "hello, world");
    assert_eq!(data.1, vec![1, 2, 3, 4]);
}

#[test]
fn ui_or_patterns_and_at_bindings() {
    fn classify(x: i32) -> (&'static str, i32) {
        match x {
            n @ (1 | 2 | 3) => ("small", n * 10),
            n @ 4..=10 if n % 2 == 0 => ("mid-even", n),
            n @ 4..=10 => ("mid-odd", n + 1),
            n @ (-10..=-1 | 11..=20) => ("outer", n.abs()),
            other => ("other", other),
        }
    }
    assert_eq!(classify(2), ("small", 20));
    assert_eq!(classify(6), ("mid-even", 6));
    assert_eq!(classify(7), ("mid-odd", 8));
    assert_eq!(classify(-5), ("outer", 5));
    assert_eq!(classify(15), ("outer", 15));
    assert_eq!(classify(99), ("other", 99));
}

#[test]
fn ui_slice_patterns_and_subslice_bindings() {
    fn summarize(s: &[i32]) -> i32 {
        match s {
            [] => 0,
            [only] => *only,
            [first, last] => first * 10 + last,
            [first, middle @ .., last] => first * 100 + middle.iter().sum::<i32>() * 10 + last,
        }
    }
    assert_eq!(summarize(&[]), 0);
    assert_eq!(summarize(&[7]), 7);
    assert_eq!(summarize(&[3, 4]), 34);
    assert_eq!(summarize(&[1, 2, 3, 4, 5]), 195);
}

#[test]
fn ui_guard_side_effects_and_fallthrough() {
    let mut checks = 0;
    let mut val = Some(10);
    let res = match val {
        Some(x)
            if {
                checks += 1;
                x > 20
            } =>
        {
            1
        }
        Some(x)
            if {
                checks += 1;
                x == 10
            } =>
        {
            val = Some(x + 5);
            2
        }
        _ => 3,
    };
    assert_eq!(res, 2);
    assert_eq!(checks, 2);
    assert_eq!(val, Some(15));
}

#[test]
fn ui_niche_optimized_enums_and_nested_options() {
    let a: Option<NonZeroU32> = NonZeroU32::new(42);
    let b: Option<NonZeroU32> = None;
    assert_eq!(a.map(NonZeroU32::get), Some(42));
    assert_eq!(b.map(NonZeroU32::get), None);

    let nested: Option<Option<bool>> = Some(None);
    assert!(matches!(nested, Some(None)));
    assert_ne!(nested, None);
    assert_ne!(nested, Some(Some(false)));

    let nz_neg = NonZeroI32::new(-7);
    assert_eq!(nz_neg.map(NonZeroI32::get), Some(-7));

    let opt_byte: Option<Option<NonZeroU8>> = Some(NonZeroU8::new(255));
    assert_eq!(opt_byte.flatten().map(NonZeroU8::get), Some(255));

    // Overwriting enum variant in place with a smaller/different payload must not leak stale fields.
    #[derive(Debug, PartialEq, Eq)]
    enum Multi {
        Empty,
        One(u64),
        Three(u32, String, bool),
    }
    let mut m = Multi::Three(7, String::from("abc"), true);
    assert_eq!(m, Multi::Three(7, String::from("abc"), true));
    m = Multi::One(999_999_999_999);
    assert_eq!(m, Multi::One(999_999_999_999));
    m = Multi::Empty;
    assert_eq!(m, Multi::Empty);
}

#[test]
fn ui_let_else_and_if_let_chains() {
    fn parse_pair(s: &str) -> Option<(i32, i32)> {
        let Some((lhs, rhs)) = s.split_once(':') else {
            return None;
        };
        let Ok(a) = lhs.trim().parse::<i32>() else {
            return None;
        };
        let Ok(b) = rhs.trim().parse::<i32>() else {
            return None;
        };
        if a >= 0 && b >= 0 { Some((a, b)) } else { None }
    }
    assert_eq!(parse_pair("12 : 34"), Some((12, 34)));
    assert_eq!(parse_pair("-1:5"), None);
    assert_eq!(parse_pair("bad"), None);
}

#[test]
fn ui_exhaustive_char_and_range_matching() {
    fn kind(c: char) -> u8 {
        match c {
            '0'..='9' => 1,
            'a'..='z' => 2,
            'A'..='Z' => 3,
            'α'..='ω' => 4,
            '\u{1F600}'..='\u{1F64F}' => 5,
            _ => 0,
        }
    }
    assert_eq!(kind('5'), 1);
    assert_eq!(kind('q'), 2);
    assert_eq!(kind('Z'), 3);
    assert_eq!(kind('λ'), 4);
    assert_eq!(kind('😀'), 5);
    assert_eq!(kind('!'), 0);
}
