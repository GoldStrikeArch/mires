// Zero-sized elements, zero-length rows and zero rows, on the stack and on the heap.
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use core::hint::black_box as bb;
#[derive(Clone, Copy, PartialEq)]
struct Z;
static mut DROPS: u32 = 0;
struct ZD;
impl Drop for ZD { fn drop(&mut self) { unsafe { DROPS += 1; } } }
fn n(x: usize) { print_usize(x) }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    let a = bb([[(); 3]; 2]);
    n(a.as_flattened().len()); n(a.as_flattened().iter().count()); n(a.concat().len()); n(a.iter().flatten().count());
    let (c, r) = a.as_flattened().as_chunks::<3>();
    n(c.len() * 10 + r.len());
    let mut v: Vec<[(); 3]> = Vec::new();
    for _ in 0..5 { v.push([(); 3]); }
    n(v.len()); n(v.as_flattened().len()); n(v.capacity().min(7));
    let f = v.into_flattened();
    n(f.len());
    let mut z: Vec<[Z; 2]> = vec![[Z; 2]; 4];
    z.as_flattened_mut()[5] = Z;
    z.as_flattened_mut().swap(0, 7);
    z.as_flattened_mut().reverse();
    n(z.as_flattened().len()); n(z.as_flattened().iter().filter(|x| **x == Z).count());
    let b: Box<[[Z; 2]; 3]> = Box::new([[Z; 2]; 3]);
    n(b.as_flattened().len());
    {
        let mut d: Vec<[ZD; 2]> = Vec::new();
        d.push([ZD, ZD]); d.push([ZD, ZD]); d.push([ZD, ZD]);
        d.truncate(2);
        n(unsafe { DROPS } as usize);
        let fl = d.into_flattened();
        n(fl.len());
    }
    n(unsafe { DROPS } as usize);
    { let bd: Box<[[ZD; 2]; 2]> = Box::new([[ZD, ZD], [ZD, ZD]]); n(bd.as_flattened().len()); }
    n(unsafe { DROPS } as usize);
    // zero-length rows
    let e = bb([[0u8; 0]; 5]);
    n(e.as_flattened().len()); n(e.len()); n(e.concat().len());
    let mut ev: Vec<[u32; 0]> = Vec::new();
    for _ in 0..4 { ev.push([]); }
    n(ev.len()); n(ev.as_flattened().len()); n(ev.iter().flatten().count());
    ev.as_flattened_mut().reverse();
    ev.as_flattened_mut().sort();
    let ef: Vec<u32> = ev.into_flattened();
    n(ef.len());
    let eb: Box<[[u64; 0]; 3]> = Box::new([[]; 3]);
    n(eb.as_flattened().len()); n(eb.len());
    // zero rows
    let none: [[u32; 2]; 0] = bb([]);
    n(none.as_flattened().len());
    let nv: Vec<[u32; 2]> = Vec::new();
    n(nv.as_flattened().len()); n(nv.as_flattened().iter().count());
    let (c, r) = nv.as_flattened().as_chunks::<2>();
    n(c.len() + r.len());
    let nf = nv.into_flattened();
    n(nf.len() + nf.capacity());
    let mut wc: Vec<[u32; 2]> = Vec::with_capacity(4);
    n(wc.as_flattened().len());
    wc.as_flattened_mut().reverse();
    let wf = wc.into_flattened();
    n(wf.len()); n(wf.capacity());
    let nb: Box<[[u32; 2]]> = Vec::new().into_boxed_slice();
    n(nb.as_flattened().len());
    let mut cl: Vec<[u32; 2]> = vec![[1, 2], [3, 4]];
    cl.clear();
    n(cl.as_flattened().len());
    cl.push([5, 6]);
    n(cl.as_flattened().iter().map(|x| *x as usize).sum());
}
