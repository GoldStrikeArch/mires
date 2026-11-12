#![no_std]
#![no_main]
#![allow(dead_code, static_mut_refs, unused, non_snake_case, non_upper_case_globals)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;

// Strings that look like syntax, field names that are JavaScript property names with a meaning,
// statics in modules and inside functions, lower-case and odd static names, a long ring.
struct Txt { a: &'static str, b: &'static str, c: &'static str, d: char, next: &'static Txt }
static X1: Txt = Txt { a: "a\"b\\", b: "it's `x` ${y} // not a comment }]{[", c: "line\nbreak\u{2028}\u{0}</script>", d: '\'', next: &X2 };
static X2: Txt = Txt { a: "}", b: "] // X1 = { next: X2 }", c: "", d: '"', next: &X1 };

struct Names { length: u32, constructor: u32, toString: u32, valueOf: &'static Names, hasOwnProperty: u32, prototype: u32, name: &'static str, buf: u32, off: u32, len: u32, TAG: u32, _0: u32, r#let: u32, r#static: u32 }
static N1: Names = Names { length: 1, constructor: 2, toString: 3, valueOf: &N2, hasOwnProperty: 4, prototype: 5, name: "n1", buf: 6, off: 7, len: 8, TAG: 9, _0: 10, r#let: 11, r#static: 12 };
static N2: Names = Names { length: 21, constructor: 22, toString: 23, valueOf: &N1, hasOwnProperty: 24, prototype: 25, name: "n2", buf: 26, off: 27, len: 28, TAG: 29, _0: 30, r#let: 31, r#static: 32 };

mod m1 { pub struct P { pub id: u32, pub o: &'static super::m2::P } pub static S: P = P { id: 1, o: &super::m2::S }; }
mod m2 { pub struct P { pub id: u32, pub o: &'static super::m1::P } pub static S: P = P { id: 2, o: &super::m1::S }; }

struct Me { id: u32, me: &'static Me }
static me: Me = Me { id: 3, me: &me };
static id: Me = Me { id: 4, me: &id };
static next: Me = Me { id: 5, me: &id };
static Object: Me = Me { id: 6, me: &assign };
static assign: Me = Me { id: 7, me: &Object };

fn local() -> u32 {
    struct L { id: u32, o: &'static L }
    static A: L = L { id: 8, o: &B };
    static B: L = L { id: 9, o: &A };
    A.o.id * 10 + B.o.o.id
}
fn local2() -> u32 {
    struct L { id: u32, o: &'static L }
    static A: L = L { id: 18, o: &A };
    static B: L = L { id: 19, o: &A };
    A.o.id * 100 + B.o.o.id
}

struct R { id: u32, next: &'static R }
macro_rules! ring {
    ($first:ident $($a:ident $b:ident)*) => { ring!(@go 0u32, $first, $($a $b)*); };
    (@go $n:expr, $cur:ident, $nx:ident $($rest:ident)*) => { static $cur: R = R { id: $n, next: &$nx }; ring!(@go $n + 1, $nx, $($rest)*); };
    (@go $n:expr, $cur:ident,) => { static $cur: R = R { id: $n, next: &Q00 }; };
}
ring!(Q00 Q01 Q02 Q03 Q04 Q05 Q06 Q07 Q08 Q09 Q10 Q11 Q12 Q13 Q14 Q15 Q16 Q17 Q18 Q19 Q20 Q21 Q22 Q23 Q24 Q25 Q26 Q27 Q28 Q29 Q30 Q31 Q32 Q33 Q34 Q35 Q36 Q37 Q38 Q39 Q40);
// two rings joined by one edge each way would be one component; joined one way they are two
static J1: R = R { id: 100, next: &J2 };
static J2: R = R { id: 101, next: &J3 };
static J3: R = R { id: 102, next: &J2 };
static K1: [&R; 3] = [&J1, &Q20, &J3];

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str(X1.a); print_str(X1.b); print_usize(X1.c.len()); print_u32(X1.d as u32);
    print_str(X1.next.a); print_str(X2.b); print_usize(X2.c.len()); print_u32(X1.next.d as u32);
    print_bool(core::ptr::eq(X2.next.next, &X2));
    let n = N1.valueOf;
    print_u32(n.length + n.constructor + n.toString + n.hasOwnProperty + n.prototype + n.buf + n.off + n.len + n.TAG + n._0 + n.r#let + n.r#static);
    let n = n.valueOf;
    print_u32(n.length + n.constructor + n.toString + n.hasOwnProperty + n.prototype + n.buf + n.off + n.len + n.TAG + n._0 + n.r#let + n.r#static);
    print_str(N2.valueOf.name);
    print_bool(core::ptr::eq(N1.valueOf.valueOf, &N1));
    print_u32(m1::S.o.id * 10 + m2::S.o.o.id);
    print_bool(core::ptr::eq(m1::S.o.o, &m1::S));
    print_u32(me.me.me.id * 10000 + id.me.id * 1000 + next.me.me.id * 100 + Object.me.id * 10 + assign.me.me.id);
    print_bool(core::ptr::eq(next.me, &id) && core::ptr::eq(Object.me.me, &Object));
    print_u32(local());
    print_u32(local2());
    let mut r = &Q00; let mut t = 0u32;
    for _ in 0..100 { t = t.wrapping_mul(7).wrapping_add(r.id); r = r.next; }
    print_u32(t);
    print_bool(core::ptr::eq(r, &Q18));
    print_u32(J1.next.next.next.id + K1[1].next.id * 1000 + K1[0].id * 100000);
    print_bool(core::ptr::eq(K1[2].next, K1[0].next));
}
