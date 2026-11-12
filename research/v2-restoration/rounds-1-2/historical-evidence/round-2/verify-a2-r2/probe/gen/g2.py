# g2.py: byte-row, overwrite, as_chunks, hand-over and address probes -> src/, names in r2b.txt
import os
P = {}


def add(name, body, pre=''):
    P[name] = 'include!("fam_common.rs");\n' + pre + '#[unsafe(no_mangle)]\npub fn rust_entry() {\n' + body + '}\n'


DIG = '''use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::String;
#[inline(never)]
fn digest(i: u32) -> [u8; 32] {
    let mut d = [0u8; 32];
    let mut x = i.wrapping_mul(2654435761).wrapping_add(12345);
    for b in d.iter_mut() { x ^= x << 13; x ^= x >> 17; x ^= x << 5; *b = x as u8; }
    d
}
#[inline(never)]
fn key4(i: u32) -> [u8; 4] { let x = i.wrapping_mul(2654435761); [(x >> 24) as u8, (x >> 16) as u8, (x >> 8) as u8, x as u8] }
'''

# ---- hashing-style code over Vec<[u8; 32]> ----
add('hb1', '''    let mut v: Vec<[u8; 32]> = Vec::new();
    for i in 0..200u32 { v.push(digest(i % 150)); }
    out(1, hr(&v));
    out(2, (v[3] == v[153]) as u64 + (v[3] == v[4]) as u64 * 2 + (v[10] < v[11]) as u64 * 4 + (v[10].cmp(&v[11]) as i32 + 1) as u64 * 8);
    v.sort();
    out(3, hr(&v));
    v.dedup();
    out(4, hr(&v) ^ v.len() as u64);
    let k = digest(77);
    out(5, v.binary_search(&k).map(|i| i as u64).unwrap_or(9999));
    out(6, v.contains(&k) as u64 + v.contains(&[0u8; 32]) as u64 * 2);
    out(7, v.iter().position(|d| *d == k).unwrap_or(9999) as u64);
    v.sort_unstable_by(|a, b| b.cmp(a));
    out(8, hr(&v));
    v.sort_by_key(|d| d[5]);
    out(9, hr(&v));
    { let mx = v.iter().max().unwrap(); let mn = v.iter().min().unwrap(); out(10, hs(mx) ^ hs(mn).rotate_left(7)); }
    let w = v.clone();
    out(11, (w == v) as u64 + (w[..3] == v[1..4]) as u64 * 2 + (w < v) as u64 * 4);
    v.retain(|d| d[0] & 1 == 0);
    out(12, hr(&v) ^ v.len() as u64);
    v.reverse();
    v.rotate_left(3);
    v.swap(0, 5);
    out(13, hr(&v));
    let c = v.concat();
    out(14, hs(&c) ^ c.len() as u64);
    out(15, hs(v.as_flattened()));
    v[2] = [7u8; 32];
    v[3][31] = 9;
    let r: [u8; 32] = v[3];
    out(16, hs(&r) ^ hr(&v));
    for d in v.iter_mut() { d.reverse(); d[0] ^= 0xff; }
    out(17, hr(&v));
    out(18, v.windows(2).filter(|w| w[0] < w[1]).count() as u64);
    out(19, v.chunks(3).map(|c| hr(c)).fold(0, |a, b| a ^ b.rotate_left(1)));
    let (a, b) = v.split_at(v.len() / 2);
    out(20, hr(a) ^ hr(b).rotate_left(3));
    let firsts: Vec<[u8; 4]> = v.iter().map(|d| *d.first_chunk::<4>().unwrap()).collect();
    out(21, hr(&firsts));
    let lasts: Vec<[u8; 4]> = v.iter().map(|d| { let t: [u8; 4] = d[28..].try_into().unwrap(); t }).collect();
    out(22, hr(&lasts));
    let m: u32 = v.iter().map(|d| (d[0] as u32) | (d[1] as u32) << 8 | (d[2] as u32) << 16 | (d[3] as u32) << 24).fold(0, |a, b| a.wrapping_add(b));
    out(23, m as u64);
    v.as_flattened_mut().fill(1);
    v[1][1] = 2;
    out(24, hr(&v));
    out(25, v.starts_with(&[[1u8; 32]]) as u64 + v.ends_with(&[[1u8; 32]]) as u64 * 2 + w.starts_with(&w[..2]) as u64 * 4);
    let mut x = w.clone();
    x.extend_from_slice(&w[..5]);
    x.sort_unstable();
    x.dedup();
    out(26, hr(&x) ^ x.len() as u64 ^ (x == w) as u64);
    let found = w.iter().filter(|d| x.binary_search(d).is_ok()).count();
    out(27, found as u64);
    let bx: Box<[[u8; 32]]> = x.into_boxed_slice();
    out(28, hr(&bx));
    let one: Box<[u8; 32]> = Box::new(digest(5));
    out(29, hs(&one[..]) ^ (*one == digest(5)) as u64 ^ (one.as_slice() == &w[0][..]) as u64);
''', DIG)
add('hb2', '''    let mut m: BTreeMap<[u8; 4], u32> = BTreeMap::new();
    for i in 0..60u32 { *m.entry(key4(i % 40)).or_insert(0) += i; m.insert([i as u8, 0, 0, 1], i); }
    out(30, m.len() as u64);
    out(31, m.iter().fold(7u64, |a, (k, v)| a.wrapping_mul(31).wrapping_add(hs(k)).wrapping_add(*v as u64)));
    out(32, m.get(&[5, 0, 0, 1]).copied().unwrap_or(999) as u64);
    out(33, m.range([3, 0, 0, 0]..[9, 0, 0, 0]).count() as u64);
    m.remove(&[5, 0, 0, 1]);
    let ks: Vec<[u8; 4]> = m.keys().copied().collect();
    out(34, hr(&ks));
    let s: BTreeSet<[u8; 32]> = (0..40u32).map(|i| digest(i % 30)).collect();
    out(35, s.len() as u64);
    out(36, s.iter().fold(7u64, |a, k| a.wrapping_mul(31).wrapping_add(hs(k))));
    out(37, s.contains(&digest(3)) as u64 + s.contains(&digest(33)) as u64 * 2);
    let mut mm: BTreeMap<[u8; 32], Vec<[u8; 4]>> = BTreeMap::new();
    for i in 0..50u32 { mm.entry(digest(i % 7)).or_default().push(key4(i)); }
    out(38, mm.values().fold(7u64, |a, v| a.wrapping_mul(31).wrapping_add(hr(v))));
    let first = mm.first_key_value().map(|(k, v)| hs(k) ^ v.len() as u64).unwrap();
    out(39, first);
    let sv: Vec<[u8; 32]> = s.into_iter().collect();
    out(40, hr(&sv));
    let mut pairs: Vec<([u8; 4], u32)> = m.into_iter().collect();
    pairs.sort_by(|a, b| b.0.cmp(&a.0));
    out(41, pairs.iter().fold(7u64, |a, (k, v)| a.wrapping_mul(31).wrapping_add(hs(k)).wrapping_add(*v as u64)));
    let mut vm: BTreeMap<u32, [u8; 4]> = BTreeMap::new();
    for i in 0..30u32 { vm.insert(i * 7 % 30, key4(i)); }
    vm.get_mut(&7).unwrap()[2] = 200;
    out(42, vm.values().fold(7u64, |a, v| a.wrapping_mul(31).wrapping_add(hs(v))));
''', DIG)

# ---- UTF-8 and string code over byte rows ----
add('ut1', '''    let names: Vec<[u8; 8]> = vec![*b"alpha   ", *b"beta    ", *b"gamma\\xce\\xb1 ", *b"\\xff\\xfe      "];
    for n in &names {
        match core::str::from_utf8(n) {
            Ok(s) => { print_str(s.trim_end()); print_usize(s.chars().count()); }
            Err(e) => print_usize(e.valid_up_to()),
        }
    }
    let joined = String::from_utf8_lossy(names.as_flattened());
    print_str(&joined);
    let s = String::from_utf8(names[0].to_vec()).unwrap();
    print_str(&s);
    let mut up = names.clone();
    for n in up.iter_mut() { n.make_ascii_uppercase(); }
    print_str(core::str::from_utf8(&up[1]).unwrap());
    out(1, names[0].is_ascii() as u64 + names[2].is_ascii() as u64 * 2 + names[0].eq_ignore_ascii_case(&up[0]) as u64 * 4);
    print_str(core::str::from_utf8(&names[0][..5]).unwrap());
    let mut buf: Vec<[u8; 4]> = Vec::new();
    for ch in "h\\u{e9}llo w\\u{f6}rld \\u{2713}\\u{1F600}".chars() { let mut b = [0u8; 4]; ch.encode_utf8(&mut b); buf.push(b); }
    out(2, hr(&buf));
    let total: usize = buf.iter().map(|b| core::str::from_utf8(b).map(|s| s.trim_end_matches('\\0').len()).unwrap_or(99)).sum();
    out(3, total as u64);
    let mut st = String::new();
    for b in &buf { st.push_str(core::str::from_utf8(b).unwrap().trim_end_matches('\\0')); }
    print_str(&st);
    let (ch, rest) = "abcdefghij".as_bytes().as_chunks::<4>();
    out(4, hr(ch) ^ hs(rest));
    let a: [u8; 8] = "12345678".as_bytes().try_into().unwrap();
    out(5, hs(&a));
    let words: Vec<[u8; 4]> = "the quick brown fox ".as_bytes().chunks_exact(4).map(|c| c.try_into().unwrap()).collect();
    out(6, hr(&words));
    let back = String::from_utf8(words.concat()).unwrap();
    print_str(&back);
    let back2 = String::from_utf8(words.into_flattened()).unwrap();
    print_str(&back2);
    let mut bs = String::from("0123456789ab").into_bytes();
    { let (ch, _) = bs.as_chunks_mut::<3>(); ch[1] = *b"xyz"; ch[3][0] = b'!'; }
    print_str(core::str::from_utf8(&bs).unwrap());
    let rows: [[u8; 3]; 2] = [*b"abc", *b"def"];
    print_str(core::str::from_utf8(rows.as_flattened()).unwrap());
    let bx: Box<[[u8; 3]; 2]> = Box::new(rows);
    print_str(core::str::from_utf8(bx.as_flattened()).unwrap());
    let cmp = names[0].as_slice().cmp(b"alpha".as_slice()) as i32;
    print_i32(cmp);
    out(7, names.iter().filter(|n| n.starts_with(b"be") || n.ends_with(b"  ")).count() as u64);
    out(8, names.iter().map(|n| n.iter().position(|b| *b == b' ').unwrap_or(8)).sum::<usize>() as u64);
''', DIG)

# ---- copy_from_slice between a byte row and an ordinary slice ----
CPY = '''#[inline(never)]
fn cpy(t: u32, v: &mut [[u8; 4]]) {
    let src = bb([1u8, 2, 3, 4]);
    v[1].copy_from_slice(&src);
    out(t + 1, hr(v));
    let s2: Vec<u8> = bb(vec![5, 6, 7, 8, 9]);
    v[2].copy_from_slice(&s2[1..]);
    out(t + 2, hr(v));
    let mut dst = [0u8; 4];
    dst.copy_from_slice(&v[1]);
    out(t + 3, hs(&dst));
    let mut d2 = vec![0u8; 6];
    d2[1..5].copy_from_slice(&v[2]);
    out(t + 4, hs(&d2));
    v[0][1..3].copy_from_slice(&d2[2..4]);
    v[3][..2].copy_from_slice(&[21, 22]);
    out(t + 5, hr(v));
    { let (a, b) = v.split_at_mut(1); a[0].copy_from_slice(&b[0]); b[1].clone_from_slice(&a[0]); }
    out(t + 6, hr(v));
    v.as_flattened_mut()[2..10].copy_from_slice(&[7u8, 8, 9, 10, 11, 12, 13, 14]);
    out(t + 7, hr(v));
    let row3 = v[3];
    v[0] = row3;
    v[1].swap_with_slice(&mut dst);
    out(t + 8, hr(v) ^ hs(&dst));
    v.copy_within(1..3, 0);
    v[3].copy_within(1..3, 0);
    out(t + 9, hr(v));
    unsafe { core::ptr::copy_nonoverlapping(v[1].as_ptr(), d2.as_mut_ptr(), 4); core::ptr::copy_nonoverlapping(d2.as_ptr().add(2), v[2].as_mut_ptr(), 3); }
    out(t + 10, hr(v) ^ hs(&d2));
    let mut big = [0u8; 16];
    big.copy_from_slice(v.as_flattened());
    big[5] = 99;
    v.as_flattened_mut().copy_from_slice(&big);
    out(t + 11, hr(v));
    let mut tmp = [[0u8; 4]; 2];
    tmp.copy_from_slice(&v[1..3]);
    tmp[0][0] = 50;
    v[2..].copy_from_slice(&tmp);
    out(t + 12, hr(v) ^ hr(&tmp));
    let mut hv: Vec<[u8; 4]> = Vec::new();
    hv.extend_from_slice(v);
    hv[1..3].copy_from_slice(&tmp);
    v[..2].copy_from_slice(&hv[2..]);
    out(t + 13, hr(v) ^ hr(&hv));
    v[0].fill(3);
    v[1][1..].fill(4);
    core::mem::swap(&mut v[0], &mut tmp[1]);
    v.swap(1, 2);
    out(t + 14, hr(v) ^ hr(&tmp));
    let r = core::mem::replace(&mut v[3], [8; 4]);
    let tk = core::mem::take(&mut v[2]);
    out(t + 15, hr(v) ^ hs(&r) ^ hs(&tk).rotate_left(9));
}
'''
add('cp1', '''    let mut st = bb([[0u8; 4]; 4]);
    cpy(0, &mut st);
    let mut v: Vec<[u8; 4]> = vec![[0; 4]; 3];
    v.push([9; 4]);
    cpy(100, &mut v);
    let mut b: Box<[[u8; 4]; 4]> = Box::new([[0; 4]; 4]);
    cpy(200, &mut b[..]);
    let mut p: Vec<[u8; 4]> = Vec::new();
    for i in 0..6u8 { p.push([i; 4]); }
    cpy(300, &mut p[1..5]);
    out(399, hr(&p));
    let mut bs: Box<[[u8; 4]]> = p.into_boxed_slice();
    cpy(400, &mut bs[2..]);
    let mut bytes = bb(vec![0u8; 19]);
    { let (ch, _) = bytes[2..].as_chunks_mut::<4>(); cpy(500, ch); }
    out(599, hs(&bytes));
''', CPY)

# ---- byte-like elements behind transparent wrappers, rows of other one-byte types ----
add('bw1', '''    let mut v: Vec<[Wrapping<u8>; 4]> = Vec::new();
    for i in 0..5u8 { v.push([Wrapping(i), Wrapping(i + 1), Wrapping(250), Wrapping(i * 3)]); }
    for r in v.iter_mut() { r[2] += Wrapping(10); }
    out(1, v.iter().flatten().fold(7u64, |a, x| a * 31 + x.0 as u64));
    out(2, v.as_flattened().iter().fold(7u64, |a, x| a * 31 + x.0 as u64));
    v.sort_by(|a, b| b[3].cmp(&a[3]));
    v.as_flattened_mut()[5] = Wrapping(77);
    out(3, v.iter().flatten().fold(7u64, |a, x| a * 31 + x.0 as u64));
    let f: Vec<Wrapping<u8>> = v.into_flattened();
    out(4, f.iter().fold(7u64, |a, x| a * 31 + x.0 as u64));
    let mut k: Vec<Key> = (0..6u8).map(|i| Key([i ^ 3, i, 9, 1])).collect();
    k.sort();
    k.dedup();
    k[1].0[2] = 5;
    out(5, k.iter().fold(7u64, |a, x| a * 31 + hs(&x.0)));
    out(6, k.binary_search(&Key([3, 0, 9, 1])).map(|i| i as u64).unwrap_or(99));
    let mut nz: Vec<[NonZeroU8; 2]> = (1..6u8).map(|i| [NonZeroU8::new(i).unwrap(), NonZeroU8::new(i * 2).unwrap()]).collect();
    nz.reverse();
    nz.as_flattened_mut()[3] = NonZeroU8::new(200).unwrap();
    out(7, nz.iter().flatten().fold(7u64, |a, x| a * 31 + x.get() as u64));
    let mut on: Vec<[Option<NonZeroU8>; 3]> = (0..5u8).map(|i| [NonZeroU8::new(i), None, NonZeroU8::new(i + 1)]).collect();
    on.as_flattened_mut()[4] = NonZeroU8::new(9);
    out(8, on.iter().flatten().fold(7u64, |a, x| a * 31 + x.map(|n| n.get() as u64).unwrap_or(1000)));
    let mut ib: Vec<[i8; 4]> = (0..5i8).map(|i| [i, -i, i * 20, -128]).collect();
    ib.sort();
    ib.as_flattened_mut().reverse();
    out(9, hr(&ib));
    let ub: Vec<[u8; 4]> = ib.iter().map(|r| [r[0] as u8, r[1] as u8, r[2] as u8, r[3] as u8]).collect();
    out(10, hr(&ub));
    let mut bo: Vec<[bool; 3]> = (0..5u8).map(|i| [i & 1 == 0, i & 2 == 0, i > 2]).collect();
    bo.sort();
    bo.dedup();
    out(11, hr(&bo) ^ bo.len() as u64);
    let mut ord: Vec<[core::cmp::Ordering; 2]> = vec![[core::cmp::Ordering::Less, core::cmp::Ordering::Greater]; 3];
    ord.as_flattened_mut()[2] = core::cmp::Ordering::Equal;
    out(12, ord.iter().flatten().fold(7u64, |a, x| a * 31 + (*x as i8 + 1) as u64));
    let mut ch: Vec<[u8; 1]> = "hello".bytes().map(|b| [b]).collect();
    ch.sort();
    ch.dedup();
    out(13, hr(&ch) ^ hs(ch.as_flattened()));
    let s = String::from_utf8(ch.into_flattened()).unwrap();
    print_str(&s);
''', '''use alloc::string::String;
use core::num::{NonZeroU8, Wrapping};
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key([u8; 4]);
''')

# ---- overwrite: aliasing, nested, destructors ----
add('ow1', '''    // *a = a.clone(), x = [x[1], x[0]], swaps through a temporary
    let mut a = bb([1u32, 2, 3]);
    { let r = &mut a; *r = r.clone(); }
    out(1, hs(&a));
    let mut x = bb([10u32, 20]);
    x = [x[1], x[0]];
    out(2, hs(&x));
    let mut y = bb([5u32, 6]);
    let t = x; x = y; y = t;
    out(3, hs(&x) ^ hs(&y).rotate_left(3));
    // nested arrays assigned whole; rows stay independent
    let mut g = bb([[1u32, 2], [3, 4]]);
    let h = bb([[5u32, 6], [7, 8]]);
    g = h;
    g[0][0] = 99;
    out(4, hr(&g) ^ hr(&h).rotate_left(3));
    { let r = &mut g; *r = [r[1], r[0]]; }
    g[1][1] = 42;
    out(5, hr(&g));
    g[0] = g[1];
    g[1][0] = 43;
    out(6, hr(&g));
    // tuples and structs holding arrays
    let mut tu = bb(([1u8, 2], 3u32, [4u64, 5]));
    let keep = tu;
    tu = ([9, 9], 8, [7, 6]);
    tu.0 = keep.0;
    tu.2[1] = keep.2[0];
    out(7, hs(&tu.0) ^ tu.1 as u64 ^ hs(&tu.2));
    struct S { a: [u32; 2], b: [[u8; 2]; 2], c: Option<[u32; 2]> }
    let mut s = S { a: [1, 2], b: [[3, 4], [5, 6]], c: None };
    let s2 = S { a: [7, 8], b: [[9, 10], [11, 12]], c: Some([13, 14]) };
    { let r = &mut s; *r = s2; }
    s.b[1] = s.b[0];
    s.c = s.c.map(|[p, q]| [q, p]);
    out(8, hs(&s.a) ^ hr(&s.b) ^ hs(&s.c.unwrap()));
    // an array of references and of boxes, assigned over
    let (p, q, r) = (bb(1u32), bb(2u32), bb(3u32));
    let mut refs = bb([&p, &q]);
    refs = [refs[1], &r];
    out(9, (*refs[0] * 10 + *refs[1]) as u64);
    let mut bxs: [Box<u32>; 2] = [Box::new(1), Box::new(2)];
    bxs = [Box::new(*bxs[1] + 10), Box::new(*bxs[0] + 20)];
    out(10, (*bxs[0] * 100 + *bxs[1]) as u64);
    // through raw pointers
    let mut z = bb([1u32, 2, 3, 4]);
    unsafe {
        let pz: *mut [u32; 4] = &mut z;
        *pz = *pz;
        pz.write(pz.read());
        let w = pz as *mut [u32; 2];
        *w.add(1) = *w;
        w.write([8, 9]);
    }
    out(11, hs(&z));
    // element views: a row inside a longer flat buffer, assigned over itself and over its neighbour
    let mut f = bb([1u32, 2, 3, 4, 5, 6]);
    {
        let (ch, _) = f.as_chunks_mut::<2>();
        ch[0] = ch[1];
        ch[2] = ch[0];
        let c1 = ch[1];
        ch[1] = [c1[1], c1[0]];
    }
    out(12, hs(&f));
    { let a3: &mut [u32; 3] = (&mut f[1..4]).try_into().unwrap(); *a3 = [a3[2], a3[1], a3[0]]; let c = a3.clone(); *a3 = c; }
    out(13, hs(&f));
    { let whole: &mut [u32; 6] = (&mut f[..]).try_into().unwrap(); *whole = [6, 5, 4, 3, 2, 1]; }
    out(14, hs(&f));
    // uninitialised arrays assigned over initialised ones
    let mut mu = bb([MaybeUninit::new(1u32), MaybeUninit::new(2)]);
    mu = [MaybeUninit::uninit(); 2];
    mu[0].write(7);
    out(15, unsafe { mu[0].assume_init() } as u64);
    let mut e: [u32; 0] = [];
    e = [];
    out(16, e.len() as u64);
''', 'use core::mem::MaybeUninit;\n')
add('ow2', '''    // destructors: the old value dropped once, the new one owned once
    {
        let mut a: [D; 2] = [D::mk(1), D::mk(2)];
        a = [D::mk(3), D::mk(4)];
        out(1, drops() as u64); out(2, hs(&a));
        let b = a.clone();
        a = b;
        out(3, drops() as u64); out(4, live() as i64 as u64);
        let mut g: [[D; 2]; 2] = [[D::mk(5), D::mk(6)], [D::mk(7), D::mk(8)]];
        g[0] = a;
        out(5, drops() as u64);
        g[1] = g[0].clone();
        out(6, drops() as u64); out(7, hr(&g));
        { let r = &mut g; *r = [[D::mk(9), D::mk(10)], [D::mk(11), D::mk(12)]]; }
        out(8, drops() as u64); out(9, live() as i64 as u64);
        let mut v: Vec<[D; 2]> = vec![[D::mk(1), D::mk(2)], [D::mk(3), D::mk(4)]];
        v[0] = [D::mk(20), D::mk(21)];
        out(10, drops() as u64);
        let last = v.pop().unwrap();
        v[0] = last;
        out(11, drops() as u64); out(12, hr(&v));
        v.as_flattened_mut()[1] = D::mk(30);
        out(13, drops() as u64);
        let mut t: (D, [D; 1]) = (D::mk(40), [D::mk(41)]);
        t = (D::mk(42), [D::mk(43)]);
        t.1 = [D::mk(44)];
        out(14, drops() as u64); out(15, t.0.h() + hs(&t.1));
        let mut ob: Option<[D; 2]> = Some([D::mk(50), D::mk(51)]);
        ob = None;
        out(16, drops() as u64);
        ob = Some([D::mk(52), D::mk(53)]);
        if let Some(arr) = &mut ob { *arr = [D::mk(54), D::mk(55)]; core::mem::swap(&mut arr[0], &mut g[0][0]); }
        out(17, drops() as u64); out(18, live() as i64 as u64);
        let mut bx: Box<[D; 2]> = Box::new([D::mk(60), D::mk(61)]);
        *bx = [D::mk(62), D::mk(63)];
        let moved = *bx;
        out(19, drops() as u64); out(20, hs(&moved));
        let mut bg: Box<[[D; 1]; 2]> = Box::new([[D::mk(70)], [D::mk(71)]]);
        bg[1] = [D::mk(72)];
        *bg = [[D::mk(73)], [D::mk(74)]];
        out(21, drops() as u64); out(22, hr(&bg[..]));
    }
    out(23, drops() as u64); out(24, live() as i64 as u64);
''')

# ---- as_chunks over truly flat buffers (the guard that became a pass) ----
add('ch1', '''    let values = bb([1i32, 2, 3, 4, 5]);
    let (chunks, rest) = values.as_chunks::<2>();
    out(1, chunks.len() as u64 * 10 + rest.len() as u64);
    out(2, hr(chunks) ^ hs(rest));
    let (rest, chunks) = values.as_rchunks::<2>();
    out(3, hr(chunks) ^ hs(rest));
    let v: Vec<u32> = (0..11u32).map(|i| i * 3).collect();
    let (c3, r3) = v.as_chunks::<3>();
    out(4, hr(c3) ^ hs(r3) ^ hs(c3.as_flattened()));
    let (c3b, _) = v[1..].as_chunks::<3>();
    out(5, hr(c3b) ^ c3b.len() as u64);
    out(6, c3b[2][1] as u64 + c3[3][2] as u64);
    let (c1, _) = v.as_chunks::<1>();
    out(7, hr(c1) ^ c1.len() as u64);
    let (c11, r11) = v.as_chunks::<11>();
    out(8, hr(c11) ^ r11.len() as u64);
    let (c12, r12) = v.as_chunks::<12>();
    out(9, c12.len() as u64 * 100 + r12.len() as u64);
    let e: [u32; 0] = [];
    let (ce, re) = e.as_chunks::<2>();
    out(10, ce.len() as u64 + re.len() as u64);
    let big: Vec<u64> = (0..9u64).map(|i| i << 33).collect();
    let (cb, rb) = big.as_chunks::<4>();
    out(11, hr(cb) ^ hs(rb));
    let fl: Vec<f64> = (0..7u32).map(|i| i as f64 * 0.5).collect();
    let (cf, rf) = fl[2..].as_chunks::<2>();
    out(12, hr(cf) ^ hs(rf));
    let ps: Vec<P> = (0..7u32).map(P::mk).collect();
    let (cp, rp) = ps.as_chunks::<3>();
    out(13, hr(cp) ^ hs(rp));
    let row: [P; 3] = cp[1];
    out(14, hs(&row));
    let refs: Vec<&'static u32> = (0..6u32).map(<&'static u32>::mk).collect();
    let (cr, _) = refs.as_chunks::<2>();
    out(15, hr(cr));
    // chunks of chunks, and a round trip back to the flat slice
    let (c2, _) = v.as_chunks::<2>();
    let (c22, r22) = c2.as_chunks::<2>();
    out(16, c22.len() as u64 * 10 + r22.len() as u64);
    out(17, c22.iter().fold(7u64, |a, g| a.wrapping_mul(31).wrapping_add(hr(g))) ^ hr(r22));
    out(18, hs(c2.as_flattened()) ^ hs(c22.as_flattened().as_flattened()));
    out(19, (c2.as_flattened().as_ptr() == v.as_ptr()) as u64);
    let it: u64 = v.chunks_exact(2).map(|c| { let a: &[u32; 2] = c.try_into().unwrap(); (a[0] * 7 + a[1]) as u64 }).sum();
    out(20, it);
    out(21, v.first_chunk::<4>().map(|c| hs(c)).unwrap_or(0) ^ v.last_chunk::<4>().map(|c| hs(c)).unwrap_or(0) ^ v.split_first_chunk::<3>().map(|(c, r)| hs(c) ^ hs(r)).unwrap_or(0));
    // real rows viewed flat, then chunked at another width
    let rows = bb([[1u32, 2], [3, 4], [5, 6], [7, 8], [9, 10], [11, 12]]);
    let (w3, _) = rows.as_flattened().as_chunks::<3>();
    out(22, hr(w3));
    let (w2, _) = rows.as_flattened()[2..].as_chunks::<2>();
    out(23, hr(w2) ^ (w2.as_ptr() as usize == rows[1..].as_ptr() as usize) as u64);
    let hv: Vec<[u32; 2]> = rows.to_vec();
    let (h3, _) = hv.as_flattened().as_chunks::<3>();
    out(24, hr(h3));
    let (h4, hr4) = hv.as_flattened()[1..].as_chunks::<4>();
    out(25, hr(h4) ^ hs(hr4));
''')
add('ch2', '''    // writes through chunks of flat buffers
    let mut v: Vec<u32> = (0..11u32).map(|i| i * 3).collect();
    { let (c, r) = v.as_chunks_mut::<3>(); c[0][1] = 100; c[2] = [7, 8, 9]; r[0] = 55; c.swap(0, 1); c[1].reverse(); }
    out(1, hs(&v));
    { let (c, _) = v[1..].as_chunks_mut::<2>(); c.reverse(); c[0] = c[1]; for x in c.iter_mut() { x[0] += 1; } }
    out(2, hs(&v));
    { let (c, _) = v.as_chunks_mut::<2>(); c.sort_by(|a, b| b[0].cmp(&a[0])); }
    out(3, hs(&v));
    { let (c, _) = v.as_chunks_mut::<2>(); c.as_flattened_mut()[3] = 1000; let (x, y) = c.split_at_mut(2); core::mem::swap(&mut x[0], &mut y[1]); x[1].copy_from_slice(&y[0]); }
    out(4, hs(&v));
    { let (c, _) = v.as_chunks_mut::<5>(); c.fill([1, 2, 3, 4, 5]); c[1][4] = 6; }
    out(5, hs(&v));
    { let (_, c) = v.as_rchunks_mut::<4>(); c[0][0] = 9; c[1] = [4, 3, 2, 1]; }
    out(6, hs(&v));
    let mut st = bb([1u64, 2, 3, 4, 5, 6, 7]);
    { let (c, _) = st.as_chunks_mut::<3>(); c[1] = c[0]; c[0][2] = 1 << 40; c.rotate_left(1); }
    out(7, hs(&st));
    let mut ps: Vec<P> = (0..7u32).map(P::mk).collect();
    { let (c, _) = ps.as_chunks_mut::<3>(); c[1] = c[0]; c[0][2].a = 77; c[1][0].b = 9; c.swap(0, 1); }
    out(8, hs(&ps));
    let mut ds: Vec<D> = (0..6u32).map(D::mk).collect();
    { let (c, _) = ds.as_chunks_mut::<2>(); c[1] = [D::mk(50), D::mk(51)]; c.swap(0, 2); c[0][1] = D::mk(52); }
    out(9, hs(&ds)); out(10, drops() as u64);
    drop(ds);
    out(11, live() as i64 as u64);
    let mut bx: Box<[u32]> = (0..8u32).collect();
    { let (c, _) = bx.as_chunks_mut::<4>(); c[1][3] = 44; let t = c[0]; c[0] = c[1]; c[1] = t; }
    out(12, hs(&bx));
    for ch in v.chunks_exact_mut(2) { let a: &mut [u32; 2] = ch.try_into().unwrap(); *a = [a[1], a[0] + 1]; }
    out(13, hs(&v));
    let mut by = bb([0u8; 10]);
    { let (c, r) = by.as_chunks_mut::<4>(); c[1] = [1, 2, 3, 4]; c[0][3] = 9; r[1] = 7; c.swap(0, 1); }
    out(14, hs(&by));
''')

# ---- hand-over: slice methods on the flattened view of heap rows, then the rows again ----
SW = '''#[inline(never)]
fn sweep<T: El + Copy + PartialOrd, const N: usize>(t: u32, rows: &mut [[T; N]]) {
    let len = rows.len() * N;
    rows.as_flattened_mut().rotate_left(1);
    out(t + 1, hr(rows));
    rows.as_flattened_mut().rotate_right(3);
    out(t + 2, hr(rows));
    { let f = rows.as_flattened_mut(); let (a, b) = f.split_at_mut(len / 2); core::mem::swap(&mut a[0], &mut b[0]); }
    out(t + 3, hr(rows));
    { let f = rows.as_flattened_mut(); f.select_nth_unstable_by(len / 2, |a, b| a.partial_cmp(b).unwrap()); }
    out(t + 4, hr(rows));
    { let f = rows.as_flattened_mut(); let mut i = 0u32; f.fill_with(|| { i += 1; T::mk(i) }); }
    out(t + 5, hr(rows));
    { let f = rows.as_flattened_mut(); unsafe { core::ptr::swap(f.as_mut_ptr(), f.as_mut_ptr().add(len - 1)); let v = core::ptr::replace(f.as_mut_ptr().add(1), T::mk(70)); f.as_mut_ptr().add(2).write(v); } }
    out(t + 6, hr(rows));
    { let f = rows.as_flattened_mut(); let (a, b) = f.split_at_mut(len / 2); let n = a.len().min(b.len()); unsafe { core::ptr::swap_nonoverlapping(a.as_mut_ptr(), b.as_mut_ptr(), n.min(2)); } }
    out(t + 7, hr(rows));
    { let f = rows.as_flattened_mut(); let (a, b) = f.split_at_mut(len / 2); let n = a.len().min(b.len()); a[..n].swap_with_slice(&mut b[..n]); }
    out(t + 8, hr(rows));
    out(t + 9, hs(rows.as_flattened()));
}
'''
for ty, n in [('u32', 2), ('u8', 4), ('u64', 2), ('f64', 3), ('u16', 2), ('i8', 2), ('u32', 1)]:
    add(f'sw_{ty}_{n}', f'''    let mut v: Vec<[{ty}; {n}]> = (0..6u32).map(|i| mkrow(i * 2)).collect();
    sweep(0, &mut v);
    out(50, hr(&v));
    v.push(mkrow(1));
    out(51, hr(&v) ^ hs(v.as_flattened()));
    let mut b: Box<[[{ty}; {n}]; 4]> = Box::new(mkgrid(3));
    sweep(100, &mut b[..]);
    out(150, hr(&b[..]));
    let mut g: [[{ty}; {n}]; 4] = bb(mkgrid(3));
    sweep(200, &mut g);
    out(250, hr(&g));
''', SW)

# rows handed over to their elements, then asked for as rows again
add('ho1', '''    // into_flattened consumes the owner; the result regrouped in place through the allocator API
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<u32> = v.into_flattened();
    out(1, hs(&f));
    let (p, len, cap) = (f.as_mut_ptr(), f.len(), f.capacity());
    core::mem::forget(f);
    let back: Vec<[u32; 2]> = unsafe { Vec::from_raw_parts(p as *mut [u32; 2], len / 2, cap / 2) };
    out(2, hr(&back));
''')
add('ho2', '''    // the flattened view borrowed, its bytes round-tripped, then the rows read again
    let mut v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    {
        let f = v.as_flattened_mut();
        let p = f.as_mut_ptr() as *mut u8;
        let q = p as *mut u32;
        unsafe { *q.add(3) = 99; }
    }
    out(1, v[1][1] as u64);
    out(2, hr(&v));
''')
add('ho3', '''    // a row slice held across a hand-over of the same block
    let mut v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let rows: *mut [u32; 2] = v.as_mut_ptr();
    let f: Vec<u32> = v.into_flattened();
    out(1, hs(&f));
    out(2, unsafe { (*rows.add(1))[1] } as u64);
''')
add('ho4', '''    // Box<[[T; N]]> of rows -> Vec -> into_flattened -> boxed -> array, and Rc/Arc from it
    let b: Box<[[u32; 2]]> = (0..4u32).map(|i| [i, i + 7]).collect();
    let f: Vec<u32> = b.into_vec().into_flattened();
    let bf: Box<[u32]> = f.into_boxed_slice();
    out(1, hs(&bf));
    let arr: Box<[u32; 8]> = bf.try_into().ok().unwrap();
    out(2, hs(&arr[..]));
    let (ch, _) = arr.as_chunks::<2>();
    out(3, hr(ch));
    let rc: Rc<[u32; 8]> = Rc::from(arr);
    out(4, hs(&rc[..]));
    let v2: Vec<[u32; 2]> = rc.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    let rc2: Rc<[[u32; 2]]> = Rc::from(v2.into_boxed_slice());
    out(5, hr(&rc2) ^ hs(rc2.as_flattened()));
    let v8: Vec<[u8; 2]> = (0..4u8).map(|i| [i, i + 7]).collect();
    let bf8: Box<[u8]> = v8.into_flattened().into_boxed_slice();
    let arr8: Box<[u8; 8]> = bf8.try_into().ok().unwrap();
    out(6, hs(&arr8[..]));
    let rc8: Rc<[u8; 8]> = Rc::from(arr8);
    out(7, hs(&rc8[..]));
    let s: Rc<[u8]> = rc8;
    out(8, hs(&s));
''')

# ---- row addresses through a reference, heap and stack, with no flattening (main comparable) ----
add('ad3', '''    let g = bb([[1u32, 2, 3], [4, 5, 6], [7, 8, 9]]);
    let base = g.as_ptr() as usize;
    print_bool(g[1].as_ptr() as usize == base + 12);
    print_bool(&g[2] as *const [u32; 3] as usize == base + 24);
    print_bool(&g[1][1] as *const u32 as usize == base + 16);
    print_u32(100);
    let v = bb(vec![[1u32, 2, 3], [4, 5, 6]]);
    let vb = v.as_ptr() as usize;
    print_bool(v[1].as_ptr() as usize == vb + 12);
    print_bool(&v[1] as *const [u32; 3] as usize == vb + 12);
    print_bool(&v[1][2] as *const u32 as usize == vb + 20);
    print_bool(core::ptr::eq(&v[1][0], v[1].as_ptr()));
    print_u32(200);
    let b: Box<[[u32; 3]; 2]> = Box::new(bb([[1, 2, 3], [4, 5, 6]]));
    let bb0 = b.as_ptr() as usize;
    print_bool(b[1].as_ptr() as usize == bb0 + 12);
    print_bool(&b[1][2] as *const u32 as usize == bb0 + 20);
    print_u32(300);
    let mut p: Vec<[u32; 3]> = Vec::new();
    for i in 0..4 { p.push([i, i, i]); }
    let pb = p.as_ptr() as usize;
    print_bool(p[3].as_ptr() as usize == pb + 36);
    print_bool(p.iter().enumerate().all(|(i, r)| r.as_ptr() as usize == pb + i * 12));
    print_u32(400);
    let v8 = bb(vec![[1u8, 2, 3], [4, 5, 6]]);
    let v8b = v8.as_ptr() as usize;
    print_bool(v8[1].as_ptr() as usize == v8b + 3);
    print_bool(&v8[1] as *const [u8; 3] as usize == v8b + 3);
    print_bool(&v8[1][2] as *const u8 as usize == v8b + 5);
    print_bool(v8.iter().enumerate().all(|(i, r)| r.as_ptr() as usize == v8b + i * 3));
    print_u32(500);
    let g8 = bb([[1u8, 2, 3], [4, 5, 6]]);
    let g8b = g8.as_ptr() as usize;
    print_bool(g8[1].as_ptr() as usize == g8b + 3);
    print_bool(&g8[1][2] as *const u8 as usize == g8b + 5);
''')

here = os.path.dirname(os.path.abspath(__file__))
src = os.path.join(here, '..', 'src')
names = sorted(P)
for n, b in P.items():
    open(os.path.join(src, f'{n}_body.rs'), 'w').write(b)
open(os.path.join(here, '..', 'r2b.txt'), 'w').write('\n'.join(names) + '\n')
print(len(names), ' '.join(names))
