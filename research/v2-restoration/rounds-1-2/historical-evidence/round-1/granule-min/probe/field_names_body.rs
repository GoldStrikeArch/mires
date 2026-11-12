// Structs whose fields are named after the keys of the runtime's own pointer records (`buf`,
// `off`, `len`, `sc`, `es`, `fo`, `an`, `pr`, `ps`, `w`, `ptr`, `meta`, `length`), holding large
// numbers. Only relations Rust guarantees are printed: an address is non-null, aligned and stable;
// casting to `*const ()` does not change it; two live objects have different addresses and are
// ordered consistently; a field's address lies inside its struct.
use core::cmp::Ordering;
use core::hint::black_box;
use core::mem::size_of;
use core::ptr;

#[repr(C)]
pub struct Slot { pub buf: u32, pub off: u32, pub len: u32 }
#[repr(C)]
pub struct Scaled { pub sc: u32, pub es: u32, pub fo: u32, pub an: u32 }
#[repr(C)]
pub struct Prefix { pub pr: u32, pub ps: u32, pub w: u32 }
#[repr(C)]
pub struct Fat { pub ptr: u32, pub meta: u32 }
#[repr(C)]
pub struct Arr { pub buf: [u8; 8], pub off: usize, pub len: usize, pub length: usize }
#[repr(C)]
pub struct Es { pub es: f64, pub sc: f64, pub off: f64 }

macro_rules! probes {
    ($whole:ident, $erased:ident, $addr:ident, $lt:ident, $eq:ident, $aeq:ident, $cmp:ident, $check:ident, $t:ty) => {
        #[inline(never)]
        fn $whole(s: &$t) -> usize { s as *const $t as usize }
        #[inline(never)]
        fn $erased(s: &$t) -> usize { (s as *const $t).cast::<()>() as usize }
        #[inline(never)]
        fn $addr(s: &$t) -> usize { (s as *const $t).addr() }
        #[inline(never)]
        fn $lt(a: &$t, b: &$t) -> bool { (a as *const $t) < (b as *const $t) }
        #[inline(never)]
        fn $eq(a: &$t, b: &$t) -> bool { ptr::eq(a, b) }
        #[inline(never)]
        fn $aeq(a: &$t, b: &$t) -> bool { ptr::addr_eq(a as *const $t, b as *const $t) }
        #[inline(never)]
        fn $cmp(a: &$t, b: &$t) -> Ordering { (a as *const $t).cmp(&(b as *const $t)) }
        #[inline(never)]
        fn $check(a: &$t, b: &$t) {
            let wa = $whole(a);
            let wb = $whole(b);
            print_bool(wa != 0 && wb != 0);
            print_bool(wa % 4 == 0 && wb % 4 == 0);
            print_bool($whole(a) == wa);
            print_bool($erased(a) == wa);
            print_bool($addr(a) == wa);
            print_bool($erased(b) == wb);
            print_bool(wa != wb);
            print_bool(wa + size_of::<$t>() <= wb || wb + size_of::<$t>() <= wa);
            print_bool($eq(a, a));
            print_bool($eq(a, b));
            print_bool($aeq(a, a));
            print_bool($aeq(a, b));
            print_bool($lt(a, b) != $lt(b, a));
            print_bool($lt(a, a));
            print_bool($lt(a, b) == (wa < wb));
            print_bool($cmp(a, b) != Ordering::Equal);
            print_bool($cmp(a, a) == Ordering::Equal);
            print_bool($cmp(a, b) == $cmp(b, a).reverse());
        }
    };
}

probes!(whole_slot, erased_slot, addr_slot, lt_slot, eq_slot, aeq_slot, cmp_slot, check_slot, Slot);
probes!(whole_scaled, erased_scaled, addr_scaled, lt_scaled, eq_scaled, aeq_scaled, cmp_scaled, check_scaled, Scaled);
probes!(whole_prefix, erased_prefix, addr_prefix, lt_prefix, eq_prefix, aeq_prefix, cmp_prefix, check_prefix, Prefix);
probes!(whole_fat, erased_fat, addr_fat, lt_fat, eq_fat, aeq_fat, cmp_fat, check_fat, Fat);
probes!(whole_arr, erased_arr, addr_arr, lt_arr, eq_arr, aeq_arr, cmp_arr, check_arr, Arr);
probes!(whole_es, erased_es, addr_es, lt_es, eq_es, aeq_es, cmp_es, check_es, Es);

// Field addresses: inside the struct, and the first field of a `repr(C)` struct at its start.
#[inline(never)]
fn slot_fields(s: &Slot) {
    let w = whole_slot(s);
    let f0 = &s.buf as *const u32 as usize;
    let f1 = &s.off as *const u32 as usize;
    let f2 = &s.len as *const u32 as usize;
    print_bool(f0 == w);
    print_bool(f1 >= w && f1 < w + 12 && f2 >= w && f2 < w + 12);
    print_bool(f0 != 0 && f1 != 0 && f2 != 0);
}
#[inline(never)]
fn scaled_fields(s: &Scaled) {
    let w = whole_scaled(s);
    let f0 = &s.sc as *const u32 as usize;
    let f1 = &s.es as *const u32 as usize;
    let f2 = &s.fo as *const u32 as usize;
    let f3 = &s.an as *const u32 as usize;
    print_bool(f0 == w);
    print_bool(f1 >= w && f1 < w + 16 && f2 >= w && f2 < w + 16 && f3 >= w && f3 < w + 16);
    print_bool(f0 % 4 == 0 && f1 % 4 == 0 && f2 % 4 == 0 && f3 % 4 == 0);
}
#[inline(never)]
fn arr_fields(s: &Arr) {
    let w = whole_arr(s);
    let b0 = s.buf.as_ptr() as usize;
    let b7 = &s.buf[7] as *const u8 as usize;
    let off = &s.off as *const usize as usize;
    let length = &s.length as *const usize as usize;
    print_bool(b0 == w);
    print_usize(b7 - b0);
    print_bool(off >= w && off < w + size_of::<Arr>());
    print_bool(length >= w && length < w + size_of::<Arr>());
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    let a = black_box(Slot { buf: 12_345, off: 1_000_000, len: 3 });
    let b = black_box(Slot { buf: 12_345, off: 1_000_000, len: 3 });
    check_slot(&a, &b);
    slot_fields(&a);
    let a = black_box(Scaled { sc: 4_000_000_000, es: 3_000_000_000, fo: 2_000_000_000, an: 5 });
    let b = black_box(Scaled { sc: 4_000_000_000, es: 3_000_000_000, fo: 2_000_000_000, an: 5 });
    check_scaled(&a, &b);
    scaled_fields(&a);
    let a = black_box(Prefix { pr: 77, ps: 4_000_000_000, w: 9 });
    let b = black_box(Prefix { pr: 77, ps: 4_000_000_000, w: 9 });
    check_prefix(&a, &b);
    let a = black_box(Fat { ptr: 5, meta: 6 });
    let b = black_box(Fat { ptr: 5, meta: 6 });
    check_fat(&a, &b);
    let a = black_box(Arr { buf: [1; 8], off: 4_000_000_000, len: 3_000_000_000, length: 4_000_000_000 });
    let b = black_box(Arr { buf: [1; 8], off: 4_000_000_000, len: 3_000_000_000, length: 4_000_000_000 });
    check_arr(&a, &b);
    arr_fields(&a);
    let a = black_box(Es { es: f64::INFINITY, sc: 1e300, off: f64::NAN });
    let b = black_box(Es { es: f64::INFINITY, sc: 1e300, off: f64::NAN });
    check_es(&a, &b);
    // Many of each alive at once, each asked every way: nothing may run out.
    let mut keep: alloc::vec::Vec<alloc::boxed::Box<Scaled>> = alloc::vec::Vec::new();
    let mut ok = true;
    let mut i = 0u32;
    while i < 3000 {
        let s = black_box(alloc::boxed::Box::new(Scaled { sc: 4_000_000_000 - i, es: 3_000_000_000, fo: i, an: 5 }));
        let w = whole_scaled(&s);
        if w == 0 || erased_scaled(&s) != w || addr_scaled(&s) != w { ok = false; }
        let f = &s.es as *const u32 as usize;
        if f < w || f >= w + 16 { ok = false; }
        keep.push(s);
        i += 1;
    }
    print_bool(ok);
    // and an ordinary allocation still works afterwards
    let v = black_box(alloc::vec![1u8, 2, 3]);
    print_bool(v.as_ptr() as usize != 0);
    print_u32(keep.len() as u32 + v[2] as u32);
}
