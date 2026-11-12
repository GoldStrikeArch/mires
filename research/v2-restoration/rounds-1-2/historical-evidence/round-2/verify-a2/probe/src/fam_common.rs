// Shared by the fam_* probes: a generic battery of row / flattened operations.
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;

pub trait El: Clone {
    fn mk(i: u32) -> Self;
    fn h(&self) -> u64;
}

static REFS: [u32; 64] = {
    let mut a = [0u32; 64];
    let mut i = 0;
    while i < 64 { a[i] = (i as u32) * 7 + 1; i += 1; }
    a
};

impl El for u8 { fn mk(i: u32) -> Self { (i * 3 + 1) as u8 } fn h(&self) -> u64 { *self as u64 } }
impl El for u16 { fn mk(i: u32) -> Self { (i * 301 + 1) as u16 } fn h(&self) -> u64 { *self as u64 } }
impl El for u32 { fn mk(i: u32) -> Self { i.wrapping_mul(0x01000193).wrapping_add(5) } fn h(&self) -> u64 { *self as u64 } }
impl El for i32 { fn mk(i: u32) -> Self { (i as i32) * -77 + 3 } fn h(&self) -> u64 { *self as i64 as u64 } }
impl El for u64 { fn mk(i: u32) -> Self { (i as u64 + 1).wrapping_mul(0x9E3779B97F4A7C15) } fn h(&self) -> u64 { *self } }
impl El for i64 { fn mk(i: u32) -> Self { -((i as i64 + 1) * 0x1_0000_0001) } fn h(&self) -> u64 { *self as u64 } }
impl El for u128 { fn mk(i: u32) -> Self { ((i as u128 + 1) << 70) | (i as u128 * 3) } fn h(&self) -> u64 { (*self as u64) ^ ((*self >> 64) as u64) } }
impl El for f32 { fn mk(i: u32) -> Self { i as f32 * 0.5 + 0.25 } fn h(&self) -> u64 { self.to_bits() as u64 } }
impl El for f64 { fn mk(i: u32) -> Self { i as f64 * 0.125 - 1.5 } fn h(&self) -> u64 { self.to_bits() } }
impl El for bool { fn mk(i: u32) -> Self { i % 3 == 1 } fn h(&self) -> u64 { *self as u64 + 1 } }
impl El for char { fn mk(i: u32) -> Self { char::from_u32(0x41 + i * 37).unwrap_or('z') } fn h(&self) -> u64 { *self as u64 } }
#[derive(Clone, Copy, PartialEq)]
pub struct P { pub a: i32, pub b: u8 }
impl El for P { fn mk(i: u32) -> Self { P { a: i as i32 * 9 - 4, b: i as u8 } } fn h(&self) -> u64 { (self.a as i64 as u64) * 257 + self.b as u64 } }
impl El for (u8, u32) { fn mk(i: u32) -> Self { (i as u8, i * 1000 + 1) } fn h(&self) -> u64 { self.0 as u64 * 65537 + self.1 as u64 } }
#[derive(Clone, Copy)]
pub enum E { A, B(u32), C { x: u8, y: i64 } }
impl El for E {
    fn mk(i: u32) -> Self { match i % 3 { 0 => E::A, 1 => E::B(i * 5), _ => E::C { x: i as u8, y: -(i as i64) } } }
    fn h(&self) -> u64 { match self { E::A => 11, E::B(v) => 100 + *v as u64, E::C { x, y } => 7000 + *x as u64 * 3 + (*y as u64) } }
}
impl El for &'static u32 { fn mk(i: u32) -> Self { &REFS[(i % 64) as usize] } fn h(&self) -> u64 { **self as u64 } }
impl El for Option<&'static u32> {
    fn mk(i: u32) -> Self { if i % 4 == 2 { None } else { Some(&REFS[(i % 64) as usize]) } }
    fn h(&self) -> u64 { match self { None => 9999, Some(r) => **r as u64 } }
}
impl El for Option<u32> {
    fn mk(i: u32) -> Self { if i % 4 == 2 { None } else { Some(i * 3) } }
    fn h(&self) -> u64 { match self { None => 9999, Some(r) => *r as u64 } }
}
// A droppy element: counts drops and clones in statics so the driver can print them.
pub static mut DROPS: u32 = 0;
pub static mut LIVE: i32 = 0;
pub struct D(pub u32);
impl Clone for D { fn clone(&self) -> Self { unsafe { LIVE += 1; } D(self.0) } }
impl Drop for D { fn drop(&mut self) { unsafe { DROPS += 1; LIVE -= 1; } } }
impl El for D { fn mk(i: u32) -> Self { unsafe { LIVE += 1; } D(i * 11 + 2) } fn h(&self) -> u64 { self.0 as u64 } }
pub fn drops() -> u32 { unsafe { DROPS } }
pub fn live() -> i32 { unsafe { LIVE } }

#[inline(never)] pub fn hs<T: El>(v: &[T]) -> u64 { v.iter().fold(7u64, |a, x| a.wrapping_mul(31).wrapping_add(x.h())) }
#[inline(never)] pub fn hr<T: El, const N: usize>(v: &[[T; N]]) -> u64 { v.iter().fold(3u64, |a, r| a.wrapping_mul(131).wrapping_add(hs(r))) }
pub fn mkrow<T: El, const N: usize>(base: u32) -> [T; N] { core::array::from_fn(|i| T::mk(base + i as u32)) }
pub fn mkgrid<T: El, const N: usize, const M: usize>(base: u32) -> [[T; N]; M] { core::array::from_fn(|r| mkrow(base + (r * N) as u32)) }
pub fn out(tag: u32, v: u64) { print_u32(tag); print_u64(v); }

// Reads only.
#[inline(never)]
pub fn reads<T: El, const N: usize>(t: u32, rows: &[[T; N]]) {
    out(t + 1, hr(rows));
    let f = rows.as_flattened();
    out(t + 2, f.len() as u64);
    out(t + 3, hs(f));
    out(t + 4, rows.iter().flatten().fold(1u64, |a, x| a.wrapping_mul(17).wrapping_add(x.h())));
    out(t + 5, hs(&rows.concat()));
    let mut c = 0u64;
    for ch in f.chunks(N.max(1)) { c = c.wrapping_mul(1009).wrapping_add(hs(ch)); }
    out(t + 6, c);
    let mut c = 0u64;
    for ch in f.chunks_exact(2) { c = c.wrapping_mul(1013).wrapping_add(hs(ch)); }
    out(t + 7, c);
    let mut c = 0u64;
    for w in f.windows(2) { c = c.wrapping_mul(1019).wrapping_add(hs(w)); }
    out(t + 8, c);
    if N > 0 && rows.len() != 1 {
        let (back, rest) = f.as_chunks::<N>();
        out(t + 9, hr(back) ^ rest.len() as u64);
    }
    let mid = f.len() / 2;
    let (l, r) = f.split_at(mid);
    out(t + 10, hs(l) ^ hs(r).rotate_left(7));
    if f.len() >= N && N > 0 {
        out(t + 11, hs(f.first_chunk::<N>().unwrap()));
        out(t + 12, hs(f.last_chunk::<N>().unwrap()));
        if f.len() >= 2 * N {
            let a: &[T; N] = (&f[N..2 * N]).try_into().unwrap();
            out(t + 13, hs(a));
        }
    }
    if f.len() > 1 {
        let odd = &f[1..];
        out(t + 14, hs(odd));
        out(t + 15, f.iter().rev().fold(1u64, |a, x| a.wrapping_mul(19).wrapping_add(x.h())));
        out(t + 16, f.iter().skip(1).step_by(2).fold(1u64, |a, x| a.wrapping_mul(23).wrapping_add(x.h())));
        out(t + 17, f.get(f.len() - 1).map(|x| x.h()).unwrap_or(0) ^ f.last().unwrap().h().rotate_left(3));
    }
    out(t + 18, f.to_vec().len() as u64 ^ hs(&f.to_vec()));
    out(t + 19, rows.len() as u64);
}

// Writes; every step is observed through both views.
#[inline(never)]
pub fn writes<T: El, const N: usize>(t: u32, rows: &mut [[T; N]]) {
    let len = rows.as_flattened().len();
    if len < 2 || N == 0 { return; }
    { let f = rows.as_flattened_mut(); f.swap(0, len - 1); f[len / 2] = T::mk(100); }
    out(t + 1, hr(rows) ^ hs(rows.as_flattened()).rotate_left(5));
    rows.as_flattened_mut().reverse();
    out(t + 2, hr(rows));
    { let f = rows.as_flattened_mut(); f.swap(0, 1); f.swap(len - 2, len - 1); }
    out(t + 3, hr(rows));
    { let f = rows.as_flattened_mut(); for i in 0..len / 2 { f.swap(i, len - 1 - i / 2); } }
    out(t + 4, hr(rows));
    rows.as_flattened_mut()[1..len - 1].fill(T::mk(5));
    out(t + 5, hr(rows));
    for (i, x) in rows.as_flattened_mut().iter_mut().enumerate() { *x = T::mk((x.h() as u32 % 13) + i as u32); }
    out(t + 6, hr(rows));
    let src: Vec<T> = (0..len as u32).map(|i| T::mk(i + 40)).collect();
    rows.as_flattened_mut().clone_from_slice(&src);
    out(t + 7, hr(rows));
    rows.as_flattened_mut().sort_by_key(|x| x.h());
    out(t + 8, hr(rows));
    rows.as_flattened_mut().sort_unstable_by_key(|x| !x.h());
    out(t + 9, hr(rows));
    rows.as_flattened_mut().sort_by(|a, b| (a.h() % 5).cmp(&(b.h() % 5)));
    out(t + 10, hr(rows));
    { let (l, r) = rows.as_flattened_mut().split_at_mut(len / 2); l[0] = T::mk(61); r[0] = T::mk(62); core::mem::swap(&mut l[l.len() - 1], &mut r[r.len() - 1]); }
    out(t + 11, hr(rows));
    for ch in rows.as_flattened_mut().chunks_mut(2) { ch[0] = T::mk(70 + ch.len() as u32); }
    out(t + 12, hr(rows));
    for ch in rows.as_flattened_mut().chunks_exact_mut(N) { ch[N - 1] = T::mk(80); }
    out(t + 13, hr(rows));
    if rows.len() != 1 { let (back, _) = rows.as_flattened_mut().as_chunks_mut::<N>(); back[0][0] = T::mk(90); let l = back.len(); back[l - 1] = mkrow(95); }
    out(t + 14, hr(rows));
    // Row-granular writes, observed flattened.
    rows[0] = mkrow(200);
    out(t + 15, hs(rows.as_flattened()));
    if rows.len() >= 2 {
        { let (l, r) = rows.split_at_mut(1); core::mem::swap(&mut l[0], &mut r[0]); }
        out(t + 19, hs(rows.as_flattened()));
        rows.swap(0, 1);
        out(t + 20, hs(rows.as_flattened()));
        rows.reverse();
        out(t + 21, hs(rows.as_flattened()));
        { let l = rows.len(); rows.swap(0, l - 1); }
        out(t + 22, hs(rows.as_flattened()));
        rows.sort_by_key(|r| hs(r));
        out(t + 23, hs(rows.as_flattened()));
        let r0 = rows[0].clone();
        rows[1] = r0;
        rows[0][0] = T::mk(330);
        out(t + 24, hs(rows.as_flattened()) ^ hr(rows));
        let l = rows.len();
        { let f = rows.as_flattened_mut(); f[N] = T::mk(340); }
        rows[l - 1].reverse();
        out(t + 25, hs(rows.as_flattened()) ^ hr(rows));
        for r in rows.iter_mut() { r[0] = T::mk(350); }
        out(t + 26, hs(rows.as_flattened()));
        rows.fill(mkrow(360));
        rows.as_flattened_mut()[N] = T::mk(361);
        out(t + 27, hs(rows.as_flattened()) ^ hr(rows));
    }
}

// Copy-only moves.
#[inline(never)]
pub fn copies<T: El + Copy, const N: usize>(t: u32, rows: &mut [[T; N]]) {
    let len = rows.as_flattened().len();
    if len < 4 || N == 0 { return; }
    rows.as_flattened_mut().copy_within(1..len - 1, 2);
    out(t + 1, hr(rows));
    rows.as_flattened_mut().copy_within(2..len, 0);
    out(t + 2, hr(rows));
    let src: Vec<T> = (0..len as u32).map(|i| T::mk(i + 21)).collect();
    rows.as_flattened_mut().copy_from_slice(&src);
    out(t + 3, hr(rows));
    rows.as_flattened_mut()[1..3].copy_from_slice(&src[len - 2..]);
    out(t + 4, hr(rows));
    unsafe {
        let p = rows.as_flattened_mut().as_mut_ptr();
        core::ptr::copy(p, p.add(1), len - 1);
    }
    out(t + 5, hr(rows));
    unsafe {
        let p = rows.as_flattened_mut().as_mut_ptr();
        core::ptr::copy_nonoverlapping(src.as_ptr(), p.add(len - 2), 2);
        core::ptr::swap(p, p.add(len - 1));
        p.add(1).write(T::mk(77));
        let v = p.add(len - 2).read();
        p.add(2).write(v);
    }
    out(t + 6, hr(rows));
    if rows.len() >= 2 {
        let r = rows[1];
        rows[0] = r;
        rows.copy_within(1.., 0);
        out(t + 7, hs(rows.as_flattened()));
        let l = rows.len();
        let (a, b) = rows.split_at_mut(l / 2);
        a[0].copy_from_slice(&b[0]);
        b[0][0] = T::mk(99);
        out(t + 8, hs(rows.as_flattened()));
        unsafe {
            let p = rows.as_mut_ptr();
            core::ptr::copy(p.add(1), p, l - 1);
            core::ptr::swap(p, p.add(l - 1));
        }
        out(t + 9, hs(rows.as_flattened()) ^ hr(rows));
    }
}

#[inline(never)]
pub fn all<T: El, const N: usize>(t: u32, rows: &mut [[T; N]]) {
    reads(t, rows);
    writes(t + 30, rows);
    reads(t + 60, rows);
}
#[inline(never)]
pub fn allc<T: El + Copy, const N: usize>(t: u32, rows: &mut [[T; N]]) {
    all(t, rows);
    copies(t + 80, rows);
    reads(t + 90, rows);
}

// Every container, for one clonable element type and row width N, four rows.
#[inline(never)]
pub fn c_a<T: El, const N: usize>(t: u32) {
    // stack
    let mut g: [[T; N]; 4] = bb(mkgrid(1));
    all(t, &mut g);
    all(t + 200, &mut g[1..3]);
    out(t + 399, hr(&g));
    // Box<[[T; N]; 4]>
    let mut b: Box<[[T; N]; 4]> = Box::new(mkgrid(2));
    all(t + 400, &mut b[..]);
    out(t + 598, hs(b.as_flattened()));
    b[2] = mkrow(7);
    out(t + 599, hs(b.as_flattened()));
    // Box<[T; N]> seen as one row
    let mut b1: Box<[T; N]> = Box::new(mkrow(3));
    out(t + 600, hs(&b1[..]));
    all(t + 600, core::slice::from_mut(&mut *b1));
    if N > 0 { b1[N - 1] = T::mk(8); }
    out(t + 799, hs(&*b1));
    // vec![row; 4]
    let mut v: Vec<[T; N]> = vec![mkrow(4); 4];
    all(t + 800, &mut v);
    v.push(mkrow(9));
    out(t + 998, hs(v.as_flattened()));
    out(t + 999, hs(&v.into_flattened()));
}
#[inline(never)]
pub fn c_b<T: El, const N: usize>(t: u32) {
    // push-built, grown from nothing
    let mut v: Vec<[T; N]> = Vec::new();
    for i in 0..5 { v.push(mkrow(i * 3)); out(t + 1000 + i, hs(v.as_flattened())); }
    all(t + 1000, &mut v);
    v.insert(1, mkrow(50));
    let popped = v.pop().unwrap();
    out(t + 1196, hs(&popped));
    v.truncate(4);
    v.extend_from_slice(&[mkrow(51), mkrow(52)]);
    out(t + 1197, hs(v.as_flattened()) ^ hr(&v));
    let removed = v.remove(0);
    out(t + 1198, hs(&removed) ^ hr(&v));
    let keep = v;
    // extend from an iterator, with_capacity(1) grown past it
    let mut v: Vec<[T; N]> = Vec::with_capacity(1);
    v.extend((0..4u32).map(|i| mkrow(i + 10)));
    all(t + 1200, &mut v);
    v.reserve(40);
    v.extend((0..3u32).map(|i| mkrow(i + 20)));
    out(t + 1398, hs(v.as_flattened()));
    v.shrink_to_fit();
    if N > 0 { v.as_flattened_mut()[N - 1] = T::mk(1); }
    out(t + 1399, hr(&v));
    // collect, then Box<[[T; N]]>, then back
    let v: Vec<[T; N]> = (0..4u32).map(|i| mkrow(i + 30)).collect();
    let mut bs: Box<[[T; N]]> = v.into_boxed_slice();
    all(t + 1400, &mut bs);
    let mut v2 = bs.into_vec();
    v2.push(mkrow(33));
    out(t + 1598, hs(v2.as_flattened()));
    // Rc<[[T; N]]> from a Vec, Rc<[[T; N]; 2]>
    let rc: Rc<[[T; N]]> = Rc::from(v2);
    reads(t + 1600, &rc);
    let rc2: Rc<[[T; N]; 2]> = Rc::new(mkgrid(5));
    reads(t + 1630, &rc2[..]);
    let mut fl = keep.into_flattened();
    fl.push(T::mk(53));
    fl.insert(0, T::mk(54));
    out(t + 1199, hs(&fl));
}
#[inline(never)]
pub fn c_c<T: El, const N: usize>(t: u32) {
    // Vec<Vec<[T; N]>>, and a struct field
    let mut vv: Vec<Vec<[T; N]>> = vec![vec![mkrow(1), mkrow(2)], Vec::new(), vec![mkrow(3); 3]];
    for (i, inner) in vv.iter_mut().enumerate() { all(t + 1700 + i as u32 * 100, inner); }
    struct S<T, const N: usize> { pad: u32, g: [[T; N]; 2], tail: [T; N] }
    let mut s: S<T, N> = S { pad: 1, g: mkgrid(6), tail: mkrow(7) };
    all(t + 2000, &mut s.g);
    out(t + 2199, hs(&s.tail) ^ s.pad as u64);
    // clone of a Vec of rows is independent
    let mut a: Vec<[T; N]> = vec![mkrow(1), mkrow(2), mkrow(3)];
    let b2 = a.clone();
    if N > 0 { a.as_flattened_mut()[0] = T::mk(44); }
    out(t + 2200, hr(&a) ^ hr(&b2).rotate_left(9));
    let mut c2: Box<[[T; N]; 2]> = Box::new(mkgrid(8));
    let d2 = c2.clone();
    if N > 0 { c2.as_flattened_mut()[N] = T::mk(45); }
    out(t + 2201, hr(&c2[..]) ^ hr(&d2[..]).rotate_left(9));
}

#[inline(never)]
pub fn containers_copy<T: El + Copy, const N: usize>(t: u32) {
    let mut g: [[T; N]; 4] = bb(mkgrid(1));
    copies(t, &mut g);
    copies(t + 10, &mut g[1..]);
    let mut b: Box<[[T; N]; 4]> = Box::new(mkgrid(2));
    copies(t + 20, &mut b[..]);
    let mut v: Vec<[T; N]> = vec![mkrow(4); 4];
    copies(t + 30, &mut v);
    let mut v: Vec<[T; N]> = Vec::new();
    for i in 0..5 { v.push(mkrow(i * 3)); }
    copies(t + 40, &mut v);
    let mut bs: Box<[[T; N]]> = v.into_boxed_slice();
    copies(t + 50, &mut bs);
    out(t + 60, hr(&g) ^ hr(&b[..]) ^ hr(&bs));
}
