//! `rustc/tests/ui`-style tests for generics, associated constants, control flow, `Cow`, `From`/`?`, and `Duration`.

use std::borrow::Cow;
use std::ops::{Deref, DerefMut};
use std::time::Duration;

#[test]
fn ui_labeled_loops_and_break_with_value() {
    let mut visited = Vec::new();
    let found = 'outer: {
        for i in 0..5 {
            for j in 0..5 {
                visited.push((i, j));
                if i * j == 6 {
                    break 'outer Some((i, j));
                }
            }
        }
        None
    };
    assert_eq!(found, Some((2, 3)));
    assert_eq!(visited.last(), Some(&(2, 3)));
}

#[test]
fn ui_mutual_recursion_and_deep_call_stack() {
    fn is_even(n: u32) -> bool {
        if n == 0 { true } else { is_odd(n - 1) }
    }
    fn is_odd(n: u32) -> bool {
        if n == 0 { false } else { is_even(n - 1) }
    }
    assert!(is_even(0));
    assert!(!is_even(1));
    assert!(is_even(200));
    assert!(is_odd(199));
}

#[derive(Debug, PartialEq, Eq)]
enum ParseError {
    Empty,
    BadDigit(char),
    Overflow,
}

impl From<char> for ParseError {
    fn from(c: char) -> Self {
        Self::BadDigit(c)
    }
}

fn parse_u32_strict(input: &str) -> Result<u32, ParseError> {
    if input.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut acc = 0u32;
    for ch in input.chars() {
        let digit = ch.to_digit(10).ok_or(ch)?;
        acc = acc
            .checked_mul(10)
            .and_then(|v| v.checked_add(digit))
            .ok_or(ParseError::Overflow)?;
    }
    Ok(acc)
}

#[test]
fn ui_question_mark_and_from_error_conversion() {
    assert_eq!(parse_u32_strict("12345"), Ok(12345));
    assert_eq!(parse_u32_strict(""), Err(ParseError::Empty));
    assert_eq!(parse_u32_strict("12x4"), Err(ParseError::BadDigit('x')));
    assert_eq!(parse_u32_strict("99999999999"), Err(ParseError::Overflow));
}

#[test]
fn ui_cow_borrowed_and_owned_transitions() {
    fn normalize(s: &str) -> Cow<'_, str> {
        if s.contains(' ') {
            Cow::Owned(s.replace(' ', "_"))
        } else {
            Cow::Borrowed(s)
        }
    }
    let a = normalize("clean_ident");
    assert!(matches!(a, Cow::Borrowed("clean_ident")));
    let b = normalize("has spaces here");
    assert!(matches!(b, Cow::Owned(_)));
    assert_eq!(b.as_ref(), "has_spaces_here");

    let mut c = normalize("plain");
    c.to_mut().push_str("_mut");
    assert_eq!(c.as_ref(), "plain_mut");
}

trait Metric {
    const SCALE: u64;
    fn raw(&self) -> u64;
    fn scaled(&self) -> u64 {
        self.raw() * Self::SCALE
    }
}

struct Kilo(u64);
impl Metric for Kilo {
    const SCALE: u64 = 1_000;
    fn raw(&self) -> u64 {
        self.0
    }
}

struct Mega(u64);
impl Metric for Mega {
    const SCALE: u64 = 1_000_000;
    fn raw(&self) -> u64 {
        self.0
    }
}

fn sum_scaled<M: Metric>(items: &[M]) -> u64 {
    items.iter().map(Metric::scaled).sum()
}

#[test]
fn ui_generics_and_associated_constants() {
    assert_eq!(sum_scaled(&[Kilo(2), Kilo(5)]), 7_000);
    assert_eq!(sum_scaled(&[Mega(3), Mega(4)]), 7_000_000);
}

struct Wrapper<T>(T);

impl<T> Deref for Wrapper<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> DerefMut for Wrapper<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

#[test]
fn ui_deref_and_deref_mut_coercions() {
    let mut w = Wrapper(String::from("hello"));
    assert_eq!(w.len(), 5);
    w.push_str(" deref");
    assert_eq!(&*w, "hello deref");
}

#[test]
fn ui_option_result_combinator_chains() {
    let nums = ["10", "bad", "20", "30"];
    let parsed: Vec<i32> = nums.iter().filter_map(|s| s.parse::<i32>().ok()).collect();
    assert_eq!(parsed, vec![10, 20, 30]);

    let all_ok: Result<Vec<i32>, _> = ["1", "2", "3"].iter().map(|s| s.parse::<i32>()).collect();
    assert_eq!(all_ok, Ok(vec![1, 2, 3]));

    let first_err: Result<Vec<i32>, _> = ["1", "nope", "3"]
        .iter()
        .map(|s| s.parse::<i32>())
        .collect();
    assert!(first_err.is_err());

    let transposed: Option<Result<i32, &str>> = Ok::<Option<i32>, &str>(Some(9)).transpose();
    assert_eq!(transposed, Some(Ok(9)));
}

#[test]
fn ui_derived_ord_and_tuple_comparisons() {
    #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct Version {
        major: u16,
        minor: u16,
        patch: u16,
    }
    let mut vers = vec![
        Version {
            major: 1,
            minor: 10,
            patch: 0,
        },
        Version {
            major: 1,
            minor: 2,
            patch: 9,
        },
        Version {
            major: 0,
            minor: 9,
            patch: 5,
        },
        Version {
            major: 1,
            minor: 2,
            patch: 1,
        },
    ];
    vers.sort();
    assert_eq!(
        vers,
        vec![
            Version {
                major: 0,
                minor: 9,
                patch: 5
            },
            Version {
                major: 1,
                minor: 2,
                patch: 1
            },
            Version {
                major: 1,
                minor: 2,
                patch: 9
            },
            Version {
                major: 1,
                minor: 10,
                patch: 0
            },
        ]
    );
}

#[test]
fn ui_duration_arithmetic() {
    let a = Duration::from_millis(1_500);
    let b = Duration::from_secs(2) + Duration::from_nanos(500_000_000);
    assert_eq!(a + b, Duration::from_secs(4));
    assert_eq!(b.checked_sub(a), Some(Duration::from_secs(1)));
    assert_eq!(a.checked_sub(b), None);
    assert_eq!(Duration::from_secs(3) * 4, Duration::from_secs(12));
    assert_eq!(Duration::from_secs(10) / 4, Duration::from_millis(2_500));
}

#[test]
fn ui_const_generics_matrix_transpose() {
    fn transpose<const R: usize, const C: usize>(m: [[i32; C]; R]) -> [[i32; R]; C] {
        let mut out = [[0; R]; C];
        for r in 0..R {
            for c in 0..C {
                out[c][r] = m[r][c];
            }
        }
        out
    }

    let m = [[1, 2, 3], [4, 5, 6]];
    let t = transpose(m);
    assert_eq!(t[0], [1, 4]);
    assert_eq!(t[1], [2, 5]);
    assert_eq!(t[2], [3, 6]);
}
