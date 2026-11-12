# g1.py: writes the round-2 verifier's record-attack probes into src/ and their names into r2a.txt
import os
P = {}


def add(name, body, pre=''):
    P[name] = 'include!("fam_common.rs");\n' + pre + '#[unsafe(no_mangle)]\npub fn rust_entry() {\n' + body + '}\n'


# ---- in-place collects (alloc::vec::in_place_collect) ----
add('ipc1', '''    let v: Vec<[u32; 2]> = bb(vec![[1, 2], [3, 4], [5, 6]]);
    let mut f: Vec<u32> = v.into_iter().flatten().collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); f.insert(0, 9); out(3, hs(&f));
''')
add('ipc2', '''    let mut v: Vec<[u32; 2]> = Vec::new();
    for i in 0..5u32 { v.push([i, i + 10]); }
    let mut f: Vec<u32> = v.into_iter().flatten().collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); f.insert(0, 9); out(3, hs(&f));
''')
add('ipc3', '''    let mut v: Vec<[u8; 4]> = Vec::new();
    for i in 0..5u8 { v.push([i, i + 10, i + 20, i + 30]); }
    let mut f: Vec<u8> = v.into_iter().flatten().collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); f.insert(0, 9); out(3, hs(&f));
    let v: Vec<[u8; 4]> = bb(vec![[1, 2, 3, 4], [5, 6, 7, 8]]);
    let f: Vec<u8> = v.into_iter().flatten().collect();
    out(4, hs(&f));
''')
add('ipc4', '''    let v: Vec<[u32; 2]> = bb(vec![[1, 2], [3, 4], [5, 6], [7, 8]]);
    let mut f: Vec<u32> = v.into_iter().map(|[a, b]| a * 10 + b).collect();
    out(1, hs(&f)); out(2, f.len() as u64);
    f.push(7); out(3, hs(&f));
    let mut v: Vec<[u32; 3]> = Vec::new();
    for i in 0..6u32 { v.push([i, i + 10, i + 20]); }
    let f: Vec<u32> = v.into_iter().map(|r| r[2]).collect();
    out(4, hs(&f));
''')
add('ipc5', '''    let v: Vec<[u32; 2]> = (0..8u32).map(|i| [i, i * 3]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().filter(|r| r[0] % 3 != 0).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
    let v: Vec<[u8; 4]> = (0..8u8).map(|i| [i, i * 3, 1, 2]).collect();
    let mut f: Vec<[u8; 4]> = v.into_iter().filter(|r| r[0] % 3 != 0).collect();
    out(3, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9, 9, 9]); out(4, hr(&f));
''')
add('ipc6', '''    let v: Vec<[u32; 4]> = (0..5u32).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().map(|r| [r[0], r[3]]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
''')
add('ipc7', '''    let v: Vec<[u8; 4]> = (0..5u8).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u8; 2]> = v.into_iter().map(|r| [r[0], r[3]]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
''')
add('ipc8', '''    let v: Vec<u32> = bb(vec![0x01020304, 0x05060708, 0x090a0b0c, 0x0d0e0f10]);
    let mut f: Vec<[u8; 4]> = v.into_iter().map(|x| x.to_le_bytes()).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9, 9, 9]); out(2, hr(&f));
''')
add('ipc9', '''    let v: Vec<(u32, u32)> = (0..5u32).map(|i| (i, i + 7)).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().map(|(a, b)| [a, b]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
''')
add('ipc9b', '''    let v: Vec<[u32; 2]> = (0..5u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<(u32, u32)> = v.into_iter().map(|[a, b]| (b, a)).collect();
    out(1, f.iter().fold(1u64, |h, t| h * 31 + t.0 as u64 * 7 + t.1 as u64));
    f.push((9, 9)); out(2, f.len() as u64);
''')
add('ipc10', '''    let v: Vec<[u32; 2]> = (0..5u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().map(|[a, b]| [b, a]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9]); out(2, hr(&f));
    let v: Vec<[u8; 4]> = (0..5u8).map(|i| [i, i + 7, 3, 4]).collect();
    let mut f: Vec<[u8; 4]> = v.into_iter().map(|[a, b, c, d]| [d, c, b, a]).collect();
    out(3, hr(&f) ^ hs(f.as_flattened()));
    f.push([9, 9, 9, 9]); out(4, hr(&f));
''')
add('ipc11', '''    let v: Vec<[u16; 2]> = (0..5u16).map(|i| [i, i + 7]).collect();
    let f: Vec<u16> = v.into_iter().map(|r| r[1]).collect();
    out(1, hs(&f));
    let v: Vec<[f64; 2]> = (0..5u32).map(|i| [i as f64, 0.5]).collect();
    let f: Vec<f64> = v.into_iter().flatten().collect();
    out(2, hs(&f));
    let v: Vec<[u64; 2]> = (0..5u64).map(|i| [i << 40, i]).collect();
    let f: Vec<u64> = v.into_iter().flatten().collect();
    out(3, hs(&f));
''')
add('ipc12', '''    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let f: Vec<[u32; 2]> = v.into_iter().skip(1).take(3).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let mut f: Vec<[u32; 1]> = v.into_iter().map(|r| [r[1]]).collect();
    out(2, hr(&f) ^ hs(f.as_flattened()));
    f.push([5]); out(3, hr(&f));
''')
add('ipc13', '''    let v: Vec<[u32; 1]> = (0..6u32).map(|i| [i + 7]).collect();
    let mut f: Vec<u32> = v.into_iter().map(|[a]| a + 1).collect();
    out(1, hs(&f));
    f.push(5); out(2, hs(&f));
''')
add('ipc14', '''    let v: Vec<u32> = (0..6u32).map(|i| i + 7).collect();
    let mut f: Vec<[u32; 1]> = v.into_iter().map(|a| [a + 1]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([5]); out(2, hr(&f));
''')
add('ipc15', '''    // flat_map with arrays: 1 x [u16; 4] -> 2 x [u16; 2]
    let v: Vec<[u16; 4]> = (0..4u16).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u16; 2]> = v.into_iter().flat_map(|r| [[r[0], r[1]], [r[2], r[3]]]).collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(2, hr(&f));
    let v: Vec<[u8; 4]> = (0..4u8).map(|i| [i, i + 1, i + 2, i + 3]).collect();
    let mut f: Vec<[u8; 2]> = v.into_iter().flat_map(|r| [[r[0], r[1]], [r[2], r[3]]]).collect();
    out(3, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(4, hr(&f));
''')
add('ipc16', '''    // into_flattened then in-place collect back over the handed-over block
    let v: Vec<[u32; 2]> = (0..6u32).map(|i| [i, i + 7]).collect();
    let f: Vec<u32> = v.into_flattened();
    out(1, hs(&f));
    let g: Vec<u32> = f.into_iter().map(|x| x * 2).collect();
    out(2, hs(&g));
    let h: Vec<[u32; 1]> = g.into_iter().map(|x| [x]).collect();
    out(3, hr(&h));
''')
add('ipc17', '''    // nested rows flattened in place
    let v: Vec<[[u32; 2]; 2]> = (0..4u32).map(|i| [[i, i + 1], [i + 2, i + 3]]).collect();
    let mut f: Vec<[u32; 2]> = v.into_iter().flatten().collect();
    out(1, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(2, hr(&f));
    let g: Vec<u32> = f.into_iter().flatten().collect();
    out(3, hs(&g));
    let v: Vec<[[u8; 2]; 2]> = (0..4u8).map(|i| [[i, i + 1], [i + 2, i + 3]]).collect();
    let mut f: Vec<[u8; 2]> = v.into_iter().flatten().collect();
    out(4, hr(&f) ^ hs(f.as_flattened()));
    f.push([5, 5]); out(5, hr(&f));
''')

# ---- Vec operations and conversions over rows, generic ----
VOPS = '''use alloc::collections::{BTreeMap, BinaryHeap, VecDeque};
use alloc::sync::Arc;
use core::cell::RefCell;
#[inline(never)]
fn vops<T: El, const N: usize>(t: u32) {
    let mut v: Vec<[T; N]> = (0..9u32).map(|i| mkrow(i * 5)).collect();
    let tail = v.split_off(6);
    out(t + 1, hr(&v) ^ hr(&tail).rotate_left(3));
    let d: Vec<[T; N]> = v.drain(1..3).collect();
    out(t + 2, hr(&v) ^ hr(&d).rotate_left(3));
    v.truncate(3);
    out(t + 3, hr(&v));
    v.extend_from_slice(&tail);
    out(t + 4, hr(&v) ^ hs(v.as_flattened()));
    v.retain(|r| hs(r) % 3 != 0);
    out(t + 5, hr(&v) ^ v.len() as u64);
    v.extend_from_within(..);
    v.extend_from_within(1..2);
    out(t + 6, hr(&v));
    v.dedup_by(|a, b| hs(a) == hs(b));
    out(t + 7, hr(&v) ^ v.len() as u64);
    let sr = v.swap_remove(0);
    out(t + 8, hr(&v) ^ hs(&sr));
    v.resize(8, mkrow(77));
    out(t + 9, hr(&v) ^ hs(v.as_flattened()));
    let sp: Vec<[T; N]> = v.splice(1..3, [mkrow(1), mkrow(2), mkrow(3)]).collect();
    out(t + 10, hr(&v) ^ hr(&sp));
    let mut w: Vec<[T; N]> = vec![mkrow(9); 2];
    v.append(&mut w);
    out(t + 11, hr(&v) ^ w.len() as u64);
    v.shrink_to_fit();
    v.as_flattened_mut().reverse();
    out(t + 12, hr(&v));
    v.truncate(4);
    v.shrink_to(0);
    v.reserve_exact(3);
    v.insert(0, mkrow(5));
    let rm = v.remove(1);
    out(t + 13, hr(&v) ^ hs(&rm));
    v.clear();
    out(t + 14, hs(v.as_flattened()) ^ v.len() as u64);
    v.push(mkrow(3));
    v.push(mkrow(4));
    out(t + 15, hr(&v) ^ hs(v.as_flattened()));
    // conversions
    let b: Box<[[T; N]]> = v.clone().into_boxed_slice();
    out(t + 16, hr(&b) ^ hs(b.as_flattened()));
    let rc: Rc<[[T; N]]> = Rc::from(b);
    out(t + 17, hr(&rc) ^ hs(rc.as_flattened()));
    let rc2: Rc<[[T; N]]> = Rc::from(v.clone());
    out(t + 18, hr(&rc2) ^ hs(rc2.as_flattened()));
    let rc3: Rc<[[T; N]]> = Rc::from(&v[..]);
    out(t + 19, hr(&rc3) ^ hs(rc3.as_flattened()));
    let arc: Arc<[[T; N]]> = Arc::from(v.clone());
    out(t + 20, hr(&arc) ^ hs(arc.as_flattened()));
    let mut v2: Vec<[T; N]> = rc.to_vec();
    v2.push(mkrow(6));
    out(t + 21, hr(&v2) ^ hs(v2.as_flattened()));
    let mut v3: Vec<[T; N]> = Vec::from(arc.as_ref());
    v3.as_flattened_mut().swap(0, N);
    out(t + 22, hr(&v3) ^ hr(&arc));
    let bx: Box<[[T; N]]> = Box::from(&v[..]);
    let mut v4 = bx.into_vec();
    v4.push(mkrow(8));
    v4.push(mkrow(10));
    out(t + 23, hr(&v4) ^ hs(v4.as_flattened()));
    let mut rc4: Rc<[[T; N]; 2]> = Rc::new(mkgrid(3));
    Rc::get_mut(&mut rc4).unwrap().as_flattened_mut()[N] = T::mk(55);
    out(t + 24, hr(&rc4[..]));
    let g: [[T; N]; 2] = Rc::try_unwrap(rc4).ok().unwrap();
    out(t + 25, hr(&g));
    let mut rc5: Rc<[[T; N]]> = Rc::from(v4.clone());
    Rc::get_mut(&mut rc5).unwrap().as_flattened_mut()[1] = T::mk(56);
    out(t + 26, hr(&rc5) ^ hs(rc5.as_flattened()));
    // other collections
    let mut dq: VecDeque<[T; N]> = VecDeque::from(v4.clone());
    dq.push_front(mkrow(1));
    dq.push_back(mkrow(2));
    dq.rotate_left(2);
    dq.push_front(mkrow(11));
    out(t + 27, dq.iter().fold(5u64, |a, r| a.wrapping_mul(37).wrapping_add(hs(r))));
    { let (a, b) = dq.as_slices(); out(t + 28, hr(a) ^ hr(b).rotate_left(11) ^ hs(a.as_flattened())); }
    out(t + 29, hr(dq.make_contiguous()) ^ hs(dq.make_contiguous().as_flattened()));
    let v5: Vec<[T; N]> = dq.into();
    out(t + 30, hr(&v5) ^ hs(v5.as_flattened()));
    let mut bm: BTreeMap<u32, [T; N]> = BTreeMap::new();
    for i in 0..20u32 { bm.insert((i * 7) % 20, mkrow(i)); }
    bm.remove(&7);
    out(t + 31, bm.values().fold(5u64, |a, r| a.wrapping_mul(37).wrapping_add(hs(r))));
    let vals: Vec<[T; N]> = bm.into_values().collect();
    out(t + 32, hr(&vals) ^ hs(vals.as_flattened()));
    let o: Option<Box<[T; N]>> = Some(Box::new(mkrow(4)));
    out(t + 33, o.as_ref().map(|b| hs(&b[..])).unwrap_or(0));
    let vb: Vec<Box<[T; N]>> = (0..3u32).map(|i| Box::new(mkrow(i))).collect();
    out(t + 34, vb.iter().fold(5u64, |a, r| a.wrapping_mul(37).wrapping_add(hs(&r[..]))));
    let unb: Vec<[T; N]> = vb.into_iter().map(|b| *b).collect();
    out(t + 35, hr(&unb) ^ hs(unb.as_flattened()));
    let cell: Rc<RefCell<Vec<[T; N]>>> = Rc::new(RefCell::new(unb));
    cell.borrow_mut().push(mkrow(12));
    cell.borrow_mut().as_flattened_mut()[0] = T::mk(13);
    out(t + 36, hr(&cell.borrow()) ^ hs(cell.borrow().as_flattened()));
    let bv: Box<Vec<[T; N]>> = Box::new(v5);
    let tup: (u32, Vec<[T; N]>, [T; N]) = (1, *bv, mkrow(2));
    out(t + 37, hr(&tup.1) ^ hs(&tup.2));
    let fl = tup.1.into_flattened();
    out(t + 38, hs(&fl) ^ fl.len() as u64);
    let fl2 = v4.into_flattened();
    let bxf: Box<[T]> = fl2.into_boxed_slice();
    out(t + 39, hs(&bxf));
    let fl3: Vec<T> = Rc::try_unwrap(cell).ok().unwrap().into_inner().into_flattened();
    out(t + 40, hs(&fl3));
}
'''
for ty, n in [('u32', 2), ('u8', 4), ('u8', 1), ('u64', 1), ('u8', 32), ('bool', 3), ('i8', 2), ('D', 2),
              ("&'static u32", 2), ('f64', 3), ('P', 2), ('u32', 1), ('u8', 2), ('u128', 1), ('D', 1)]:
    nm = 'vop_' + ty.replace("&'static u32", 'ref').lower() + '_' + str(n)
    extra = '    out(40000, drops() as u64);\n    out(40001, live() as i64 as u64);\n' if ty == 'D' else ''
    add(nm, f'    vops::<{ty}, {n}>(0);\n' + extra, VOPS)

# ---- MaybeUninit and new_uninit ----
add('mu1', '''    let mut b: Box<MaybeUninit<[u32; 4]>> = Box::new_uninit();
    b.write([1, 2, 3, 4]);
    let b: Box<[u32; 4]> = unsafe { b.assume_init() };
    out(1, hs(&b[..]));
    let mut b: Box<MaybeUninit<[[u32; 2]; 3]>> = Box::new_uninit();
    unsafe { b.as_mut_ptr().write([[1, 2], [3, 4], [5, 6]]); }
    let mut b = unsafe { b.assume_init() };
    b.as_flattened_mut()[3] = 9;
    out(2, hr(&b[..]) ^ hs(b.as_flattened()));
    let mut s: Box<[MaybeUninit<[u32; 2]>]> = Box::new_uninit_slice(3);
    for (i, x) in s.iter_mut().enumerate() { x.write([i as u32, 7]); }
    let mut s: Box<[[u32; 2]]> = unsafe { s.assume_init() };
    s.as_flattened_mut().reverse();
    out(3, hr(&s) ^ hs(s.as_flattened()));
    let mut v: Vec<[u32; 2]> = Vec::with_capacity(4);
    v.push([1, 2]);
    { let sp = v.spare_capacity_mut(); sp[0].write([3, 4]); sp[1].write([5, 6]); }
    unsafe { v.set_len(3); }
    out(4, hr(&v) ^ hs(v.as_flattened()));
    let mut m: MaybeUninit<[[u32; 2]; 2]> = MaybeUninit::uninit();
    unsafe {
        m.as_mut_ptr().write([[1, 2], [3, 4]]);
        (m.as_mut_ptr() as *mut u32).add(3).write(8);
        (m.as_mut_ptr() as *mut [u32; 2]).add(1).cast::<u32>().write(7);
    }
    let g = unsafe { m.assume_init() };
    out(5, hr(&g));
    let mut s: Box<[MaybeUninit<u32>]> = Box::new_uninit_slice(6);
    for (i, x) in s.iter_mut().enumerate() { x.write(i as u32 * 3); }
    let s: Box<[u32]> = unsafe { s.assume_init() };
    let (ch, _) = s.as_chunks::<2>();
    out(6, hr(ch));
''', 'use core::mem::MaybeUninit;\n')
add('mu2', '''    let mut b: Box<MaybeUninit<[u8; 4]>> = Box::new_uninit();
    b.write([1, 2, 3, 4]);
    let b: Box<[u8; 4]> = unsafe { b.assume_init() };
    out(1, hs(&b[..]));
    let mut b: Box<MaybeUninit<[[u8; 2]; 3]>> = Box::new_uninit();
    unsafe { b.as_mut_ptr().write([[1, 2], [3, 4], [5, 6]]); }
    let mut b = unsafe { b.assume_init() };
    b.as_flattened_mut()[3] = 9;
    out(2, hr(&b[..]) ^ hs(b.as_flattened()));
    let mut s: Box<[MaybeUninit<[u8; 2]>]> = Box::new_uninit_slice(3);
    for (i, x) in s.iter_mut().enumerate() { x.write([i as u8, 7]); }
    let mut s: Box<[[u8; 2]]> = unsafe { s.assume_init() };
    s.as_flattened_mut().reverse();
    out(3, hr(&s) ^ hs(s.as_flattened()));
    let mut v: Vec<[u8; 2]> = Vec::with_capacity(4);
    v.push([1, 2]);
    { let sp = v.spare_capacity_mut(); sp[0].write([3, 4]); sp[1].write([5, 6]); }
    unsafe { v.set_len(3); }
    out(4, hr(&v) ^ hs(v.as_flattened()));
    let mut m: MaybeUninit<[[u8; 2]; 2]> = MaybeUninit::uninit();
    unsafe {
        m.as_mut_ptr().write([[1, 2], [3, 4]]);
        (m.as_mut_ptr() as *mut u8).add(3).write(8);
        (m.as_mut_ptr() as *mut [u8; 2]).add(1).cast::<u8>().write(7);
    }
    let g = unsafe { m.assume_init() };
    out(5, hr(&g));
    let mut a: [MaybeUninit<[u8; 4]>; 3] = [const { MaybeUninit::uninit() }; 3];
    for (i, x) in a.iter_mut().enumerate() { x.write([i as u8 + 1; 4]); }
    let a: [[u8; 4]; 3] = unsafe { (&a as *const [MaybeUninit<[u8; 4]>; 3] as *const [[u8; 4]; 3]).read() };
    out(6, hr(&a) ^ hs(a.as_flattened()));
    let mut vm: Vec<MaybeUninit<[u8; 4]>> = Vec::with_capacity(2);
    vm.push(MaybeUninit::new([1, 2, 3, 4]));
    vm.push(MaybeUninit::new([5, 6, 7, 8]));
    out(7, vm.iter().fold(1u64, |h, m| h * 31 + hs(unsafe { m.assume_init_ref() })));
''', 'use core::mem::MaybeUninit;\n')

# ---- zero capacities, zero lengths, zero-sized ----
add('zz1', '''    let v: Vec<[u32; 2]> = Vec::new();
    out(1, hs(v.as_flattened()) ^ v.as_flattened().len() as u64);
    out(2, v.into_flattened().len() as u64);
    let mut v: Vec<[u32; 2]> = Vec::with_capacity(0);
    out(3, v.as_flattened().len() as u64);
    v.push([1, 2]);
    out(4, hs(v.as_flattened()));
    v.pop();
    out(5, v.as_flattened().len() as u64);
    v.push([3, 4]); v.push([5, 6]);
    out(6, hr(&v) ^ hs(v.as_flattened()));
    v.clear(); v.shrink_to_fit();
    out(7, v.as_flattened().len() as u64 + v.capacity() as u64);
    v.push([7, 8]);
    out(8, hr(&v) ^ hs(v.as_flattened()));
    let v: Vec<[u32; 2]> = vec![[0u32; 2]; 0];
    out(9, v.as_flattened().len() as u64);
    let b: Box<[[u32; 2]; 0]> = Box::new([]);
    out(10, b.as_flattened().len() as u64);
    let mut v: Vec<[u32; 0]> = vec![[]; 3];
    v.push([]);
    out(11, v.as_flattened().len() as u64 + v.len() as u64 * 10);
    out(12, v.into_flattened().len() as u64);
    let v: Vec<[(); 3]> = vec![[(); 3]; 2];
    out(13, v.as_flattened().len() as u64);
    let b: Box<[[u8; 0]; 5]> = Box::new([[]; 5]);
    out(14, b.as_flattened().len() as u64 + b.len() as u64 * 10);
    let e: Box<[[u32; 2]]> = Box::new([]);
    let mut ev = e.into_vec();
    ev.push([1, 2]);
    out(15, hr(&ev));
    let mut v: Vec<[u8; 4]> = Vec::new();
    out(16, v.as_flattened().len() as u64);
    v.extend(core::iter::empty::<[u8; 4]>());
    v.shrink_to_fit();
    out(17, hs(v.as_flattened()));
    v.push([1, 2, 3, 4]);
    v.pop();
    out(18, v.as_flattened().len() as u64);
    v.push([5, 6, 7, 8]);
    out(19, hr(&v) ^ hs(v.as_flattened()));
    let r: Rc<[[u32; 2]]> = Rc::from(Vec::new());
    out(20, r.as_flattened().len() as u64);
    let r: Rc<[[u8; 2]]> = Rc::from(Vec::new());
    out(21, r.as_flattened().len() as u64);
    let mut v = vec![[1u32, 2], [3, 4]];
    let d: Vec<[u32; 2]> = v.drain(..).collect();
    out(22, v.as_flattened().len() as u64 + hr(&d));
    v.push([9, 9]);
    out(23, hr(&v));
    let v8: Vec<[u8; 4]> = Vec::with_capacity(3);
    out(24, v8.as_flattened().len() as u64 + v8.capacity() as u64);
    out(25, v8.into_flattened().capacity() as u64);
    let v32: Vec<[u32; 2]> = Vec::with_capacity(3);
    out(26, v32.into_flattened().capacity() as u64);
''')

# ---- allocator API ----
AL = 'use alloc::alloc::{alloc, alloc_zeroed, dealloc, realloc, Layout};\n'
add('al1', '''    unsafe {
        let l = Layout::new::<[[u32; 2]; 3]>();
        let p = alloc(l) as *mut [u32; 2];
        for i in 0..3 { p.add(i).write([i as u32, 10 + i as u32]); }
        { let s = core::slice::from_raw_parts_mut(p, 3); out(1, hr(s) ^ hs(s.as_flattened())); }
        let p2 = realloc(p as *mut u8, l, 40) as *mut [u32; 2];
        p2.add(3).write([7, 8]);
        p2.add(4).write([9, 10]);
        { let s = core::slice::from_raw_parts_mut(p2, 5); out(2, hr(s) ^ hs(s.as_flattened())); }
        let q = p2 as *mut u32;
        out(3, *q.add(5) as u64);
        *q.add(4) = 44;
        out(4, (*p2.add(2))[0] as u64);
        dealloc(p2 as *mut u8, Layout::from_size_align_unchecked(40, 4));
        let z = alloc_zeroed(l) as *mut [u32; 2];
        out(5, (*z.add(2))[1] as u64);
        (*z.add(1))[0] = 6;
        { let s = core::slice::from_raw_parts(z, 3); out(6, hr(s) ^ hs(s.as_flattened())); }
        dealloc(z as *mut u8, l);
        let l8 = Layout::new::<[[u8; 4]; 3]>();
        let p = alloc(l8) as *mut [u8; 4];
        for i in 0..3 { p.add(i).write([i as u8, 10, 20, 30]); }
        { let s = core::slice::from_raw_parts_mut(p, 3); out(7, hr(s) ^ hs(s.as_flattened())); }
        let p2 = realloc(p as *mut u8, l8, 20) as *mut [u8; 4];
        p2.add(3).write([7, 8, 9, 10]);
        p2.add(4).write([9, 10, 11, 12]);
        { let s = core::slice::from_raw_parts_mut(p2, 5); out(8, hr(s) ^ hs(s.as_flattened())); }
        dealloc(p2 as *mut u8, Layout::from_size_align_unchecked(20, 1));
        let z = alloc_zeroed(l8) as *mut [u8; 4];
        out(9, (*z.add(2))[1] as u64);
        (*z.add(1))[0] = 6;
        { let s = core::slice::from_raw_parts(z, 3); out(10, hr(s) ^ hs(s.as_flattened())); }
        dealloc(z as *mut u8, l8);
    }
''', AL)
add('al2', '''    // one block, cast twice from its byte pointer with different element types, both pointers live
    unsafe {
        let l = Layout::new::<[[u32; 2]; 3]>();
        let raw = alloc(l);
        let rows = raw as *mut [u32; 2];
        for i in 0..3 { rows.add(i).write([i as u32 + 1, 10 + i as u32]); }
        let flat = raw as *mut u32;
        out(1, *flat.add(2) as u64);
        out(2, (*rows.add(1))[1] as u64);
        *flat.add(5) = 99;
        out(3, (*rows.add(2))[1] as u64);
        (*rows.add(0))[0] = 55;
        out(4, *flat as u64);
        let rows2 = raw as *mut [u32; 2];
        out(5, (*rows2.add(2))[0] as u64);
        dealloc(raw, l);
    }
''', AL)
add('al3', '''    // one block, rows of two widths over the same elements
    unsafe {
        let l = Layout::new::<[u32; 12]>();
        let raw = alloc(l);
        let r2 = raw as *mut [u32; 2];
        for i in 0..6 { r2.add(i).write([i as u32 * 2, i as u32 * 2 + 1]); }
        let r3 = raw as *mut [u32; 3];
        out(1, (*r3.add(1))[0] as u64);
        out(2, (*r3.add(3))[2] as u64);
        let r6 = raw as *mut [[u32; 2]; 3];
        out(3, (*r6.add(1))[2][1] as u64);
        out(4, (*r2.add(5))[1] as u64);
        dealloc(raw, l);
    }
''', AL)
add('al4', '''    // a block first used as u32s, then as rows of the same u32s (bytemuck-style regrouping)
    unsafe {
        let l = Layout::new::<[u32; 6]>();
        let raw = alloc(l);
        let flat = raw as *mut u32;
        for i in 0..6 { flat.add(i).write(i as u32 + 1); }
        let rows = raw as *mut [u32; 2];
        out(1, (*rows.add(1))[1] as u64);
        (*rows.add(2))[0] = 77;
        out(2, *flat.add(4) as u64);
        { let s = core::slice::from_raw_parts(rows, 3); out(3, hr(s) ^ hs(s.as_flattened())); }
        dealloc(raw, l);
        let raw = alloc_zeroed(l);
        let rows = raw as *mut [u32; 2];
        out(4, (*rows.add(1))[1] as u64);
        let flat = raw as *mut u32;
        out(5, *flat.add(5) as u64);
        dealloc(raw, l);
    }
''', AL)

# ---- three deep ----
add('d3', '''    let mut v: Vec<[[[u32; 2]; 2]; 2]> = Vec::new();
    for i in 0..3u32 { v.push([[[i, i + 1], [i + 2, i + 3]], [[i + 4, i + 5], [i + 6, i + 7]]]); }
    out(1, h2(v.as_flattened()) ^ hr(v.as_flattened().as_flattened()));
    out(2, hs(v.as_flattened().as_flattened().as_flattened()));
    v.as_flattened_mut().as_flattened_mut().as_flattened_mut()[13] = 99;
    out(3, v[1][1][0][1] as u64);
    v[2][0][1][0] = 77;
    out(4, hs(v.as_flattened().as_flattened().as_flattened()));
    v.as_flattened_mut()[3] = [[5, 5], [6, 6]];
    out(5, hs(v.as_flattened().as_flattened().as_flattened()));
    let f1: Vec<[[u32; 2]; 2]> = v.clone().into_flattened();
    out(6, h2(&f1) ^ hr(f1.as_flattened()));
    let f2: Vec<[u32; 2]> = f1.into_flattened();
    out(7, hr(&f2));
    let mut f3: Vec<u32> = f2.into_flattened();
    f3.push(1);
    out(8, hs(&f3));
    let b: Box<[[[u8; 2]; 2]; 2]> = Box::new([[[1, 2], [3, 4]], [[5, 6], [7, 8]]]);
    out(9, hs(b.as_flattened().as_flattened()));
    let mut w: Vec<[[[u8; 2]; 2]; 2]> = vec![*b; 2];
    w.push([[[9; 2]; 2]; 2]);
    w.as_flattened_mut().as_flattened_mut().as_flattened_mut()[9] = 42;
    out(10, w[1][0][0][1] as u64 + hs(w.as_flattened().as_flattened().as_flattened()));
    let g: Vec<u8> = w.into_flattened().into_flattened().into_flattened();
    out(11, hs(&g));
    let mut x: Vec<[[[u64; 1]; 1]; 1]> = vec![[[[5]]], [[[6]]]];
    x.push([[[7]]]);
    x.as_flattened_mut().as_flattened_mut().as_flattened_mut()[1] = 9;
    out(12, x[1][0][0][0] + hs(x.as_flattened().as_flattened().as_flattened()));
    let y: Vec<u64> = x.into_flattened().into_flattened().into_flattened();
    out(13, hs(&y));
''', 'fn h2<T: El, const N: usize, const M: usize>(v: &[[[T; N]; M]]) -> u64 { v.iter().fold(11u64, |a, g| a.wrapping_mul(17).wrapping_add(hr(g))) }\n')

here = os.path.dirname(os.path.abspath(__file__))
src = os.path.join(here, '..', 'src')
names = sorted(P)
for n, b in P.items():
    open(os.path.join(src, f'{n}_body.rs'), 'w').write(b)
open(os.path.join(here, '..', 'r2a.txt'), 'w').write('\n'.join(names) + '\n')
print(len(names), ' '.join(names))
