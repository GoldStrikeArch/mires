use crate::diff::{ws, Change, ADD, KEEP, REMOVE};
use alloc::string::{String, ToString};
fn leading(s: &str) -> &str {
    &s[..s.len() - s.trim_start_matches(ws).len()]
}
fn trailing(s: &str) -> &str {
    &s[s.trim_end_matches(ws).len()..]
}
fn common(a: &str, b: &str, suffix: bool) -> usize {
    let mut n = 0;
    if suffix {
        for (x, y) in a.chars().rev().zip(b.chars().rev()) {
            if x != y {
                break;
            }
            n += x.len_utf8()
        }
    } else {
        for (x, y) in a.chars().zip(b.chars()) {
            if x != y {
                break;
            }
            n += x.len_utf8()
        }
    }
    n
}
fn overlap(a: &str, b: &str) -> usize {
    for (i, _) in a.char_indices() {
        if b.starts_with(&a[i..]) {
            return a.len() - i;
        }
    }
    0
}
fn trim_front(s: &mut String, n: usize) {
    *s = s[n..].to_string()
}
fn trim_back(s: &mut String, n: usize) {
    s.truncate(s.len() - n)
}
fn adjust(v: &mut [Change], start: usize, del: usize, ins: usize, end: usize) {
    let none = v.len();
    if del != none && ins != none {
        let op = leading(&v[del].value);
        let np = leading(&v[ins].value);
        let ps = common(op, np, false);
        let nplen = np.len();
        let prefix = np[..ps].to_string();
        let os = trailing(&v[del].value);
        let ns = trailing(&v[ins].value);
        let ss = common(os, ns, true);
        let nslen = ns.len();
        let suffix = ns[ns.len() - ss..].to_string();
        if start != none {
            trim_back(&mut v[start].value, nplen);
            v[start].value.push_str(&prefix);
            trim_front(&mut v[del].value, ps);
            trim_front(&mut v[ins].value, ps);
        }
        if end != none {
            let tail = &v[end].value[nslen..];
            v[end].value = suffix + tail;
            trim_back(&mut v[del].value, ss);
            trim_back(&mut v[ins].value, ss);
        }
    } else if ins != none {
        if start != none {
            let n = leading(&v[ins].value).len();
            trim_front(&mut v[ins].value, n)
        }
        if end != none {
            let n = leading(&v[end].value).len();
            trim_front(&mut v[end].value, n)
        }
    } else if del != none {
        if start != none && end != none {
            let full = leading(&v[end].value).to_string();
            let a = common(&full, leading(&v[del].value), false);
            let b = common(&full[a..], trailing(&v[del].value), true);
            trim_front(&mut v[del].value, a);
            trim_back(&mut v[del].value, b);
            trim_front(&mut v[end].value, full.len() - b);
            trim_back(&mut v[start].value, b);
        } else if end != none {
            let n = overlap(trailing(&v[del].value), leading(&v[end].value));
            trim_back(&mut v[del].value, n);
        } else if start != none {
            let n = overlap(trailing(&v[start].value), leading(&v[del].value));
            trim_front(&mut v[del].value, n);
        }
    }
}
pub fn dedupe(v: &mut [Change]) {
    let none = v.len();
    let mut start = none;
    let mut del = none;
    let mut ins = none;
    for i in 0..none {
        match v[i].kind {
            ADD => ins = i,
            REMOVE => del = i,
            KEEP => {
                adjust(v, start, del, ins, i);
                start = i;
                del = none;
                ins = none;
            }
            _ => {}
        }
    }
    adjust(v, start, del, ins, none);
}
