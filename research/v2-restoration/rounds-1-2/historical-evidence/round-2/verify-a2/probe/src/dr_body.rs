// Destructors of elements inside heap blocks of rows: each dropped exactly once on every route.
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
static mut LOG: u64 = 0;
static mut N: u32 = 0;
static mut LIVE: i32 = 0;
struct D(u32);
impl D { fn new(i: u32) -> D { unsafe { LIVE += 1; } D(i) } }
impl Drop for D { fn drop(&mut self) { unsafe { LOG = LOG.wrapping_mul(1000003).wrapping_add(self.0 as u64 + 1); N += 1; LIVE -= 1; } } }
impl Clone for D { fn clone(&self) -> D { D::new(self.0 + 1000) } }
fn row(b: u32) -> [D; 2] { [D::new(b), D::new(b + 1)] }
fn st(t: u32) { print_u32(t); print_u64(unsafe { LOG }); print_u32(unsafe { N }); print_i32(unsafe { LIVE }); }
#[inline(never)] fn sum(v: &[D]) -> u32 { v.iter().map(|d| d.0).sum() }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    { let b: Box<[D; 2]> = Box::new(row(1)); print_u32(sum(&b[..])); }
    st(1);
    { let b: Box<[[D; 2]; 3]> = Box::new([row(10), row(20), row(30)]); print_u32(sum(b.as_flattened())); }
    st(2);
    { let v: Vec<[D; 2]> = vec![row(40), row(50)]; print_u32(sum(v.as_flattened())); }
    st(3);
    { let mut v: Vec<[D; 2]> = Vec::new(); for i in 0..5 { v.push(row(60 + i * 2)); } print_u32(sum(v.as_flattened())); v.truncate(3); st(4); let _ = v.pop(); st(5); let r = v.remove(0); st(6); drop(r); st(7); v.insert(0, row(90)); v[1] = row(92); st(8); }
    st(9);
    { let mut v: Vec<[D; 2]> = vec![row(100), row(110), row(120), row(130)]; let d: Vec<[D; 2]> = v.drain(1..3).collect(); st(10); drop(v); st(11); print_u32(sum(d.as_flattened())); }
    st(12);
    { let mut v: Vec<[D; 2]> = vec![row(140), row(150), row(160)]; v.clear(); st(13); v.push(row(170)); v.shrink_to_fit(); st(14); }
    st(15);
    { let v: Vec<[D; 2]> = vec![row(180), row(190)]; let f: Vec<D> = v.into_flattened(); st(16); print_u32(sum(&f)); let mut f = f; f.push(D::new(199)); let _ = f.swap_remove(0); st(17); }
    st(18);
    { let v: Vec<[D; 2]> = vec![row(200), row(210)]; let b: Box<[[D; 2]]> = v.into_boxed_slice(); st(19); print_u32(sum(b.as_flattened())); let v2 = b.into_vec(); st(20); let mut it = v2.into_iter(); let first = it.next(); st(21); drop(it); st(22); drop(first); }
    st(23);
    { let v: Vec<[D; 2]> = vec![row(220), row(230)]; let rc: Rc<[[D; 2]]> = Rc::from(v); let rc2 = rc.clone(); st(24); drop(rc); st(25); print_u32(sum(rc2.as_flattened())); }
    st(26);
    { let v: Vec<[D; 2]> = vec![row(240); 3]; st(27); let w = v.clone(); st(28); drop(v); st(29); print_u32(sum(w.as_flattened())); }
    st(30);
    { let mut v: Vec<[D; 2]> = vec![row(250), row(260), row(270)]; v.as_flattened_mut()[3] = D::new(999); st(31); v.as_flattened_mut().swap(0, 5); v.as_flattened_mut().reverse(); st(32); v.retain(|r| r[0].0 != 999 && r[1].0 != 999); st(33); v.dedup_by_key(|r| r[0].0 / 1000); st(34); }
    st(35);
    { let b: Box<[[D; 2]; 2]> = Box::new([row(280), row(290)]); let a: [[D; 2]; 2] = *b; st(36); let [x, y] = a; drop(y); st(37); drop(x); }
    st(38);
    { let v: Vec<[D; 2]> = vec![row(300), row(310)]; let a: Result<[[D; 2]; 2], _> = v.try_into(); st(39); if let Ok(a) = a { print_u32(sum(a.as_flattened())); } }
    st(40);
    { let mut v: Vec<[[D; 2]; 2]> = Vec::with_capacity(1); v.push([row(320), row(330)]); v.push([row(340), row(350)]); let mid = v.into_flattened(); st(41); let inner = mid.into_flattened(); st(42); print_u32(sum(&inner)); }
    st(43);
    { let mut v: Vec<Option<[D; 2]>> = vec![Some(row(360)), None, Some(row(370))]; let t = v[0].take(); st(44); v[1] = t; st(45); }
    st(46);
    { let mut v: Vec<[D; 2]> = vec![row(380), row(390)]; let old = core::mem::replace(&mut v[0], row(400)); st(47); v.swap(0, 1); drop(old); st(48); let (a, b) = v.split_at_mut(1); core::mem::swap(&mut a[0], &mut b[0]); st(49); }
    st(50);
}
