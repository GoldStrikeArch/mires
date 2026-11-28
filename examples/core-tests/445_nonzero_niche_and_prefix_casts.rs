//! `NonZero` niche transmutes (`NonZero<char>` + integer widths), `semver::Identifier`-shaped
//! POD pointer casts (`*const Identifier -> *const NonZero<u64>`), and multi-level offset-0
//! layout-prefix pointer casts.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use core::hint::black_box;
use core::mem::transmute;
use core::num::NonZero;
use core::ptr::{self, NonNull};

#[repr(C)]
struct SingleIdent {
    raw: NonZero<u64>,
}

/// Exact wasm32 layout of `semver::Identifier` (8 bytes: 4-byte `NonNull<u8>` + `[u8; 4]`).
#[repr(C, align(8))]
struct SemverIdent {
    head: NonNull<u8>,
    tail: [u8; 4],
}

#[repr(C)]
struct Inner {
    val: u32,
    extra: u32,
}

#[repr(C)]
struct Outer {
    inner: Inner,
    tag: u32,
}

#[repr(C)]
struct ArrayPrefix {
    arr: [u32; 4],
    tag: u32,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    // 1. NonZero<char> niche transmutes (core/src/num/nonzero.rs)
    print_bool(NonZero::<char>::new(black_box('\0')).is_none());
    let nz_a = NonZero::<char>::new(black_box('A')).unwrap();
    print_u32(nz_a.get() as u32);
    let nz_max = NonZero::<char>::new(black_box(char::MAX)).unwrap();
    print_u32(nz_max.get() as u32);
    print_bool(NonZero::<u64>::new(black_box(0)).is_none());
    print_u64(NonZero::<u64>::new(black_box(0x0102_0304_0506_0708)).unwrap().get());

    // 2. Single-scalar-field layout-identical pointer cast (`*const SingleIdent -> *const NonZero<u64>`)
    let single = black_box(SingleIdent {
        raw: NonZero::new(0x1122_3344_5566_7788u64).unwrap(),
    });
    let raw_nz = unsafe { ptr::addr_of!(single).cast::<NonZero<u64>>().read() };
    print_u64(raw_nz.get());

    // 3. Multi-field POD struct with thin pointer (`semver::Identifier` pattern)
    let inline_bytes: [u8; 8] = black_box([1, 2, 3, 4, 5, 6, 7, 8]);
    let mut ident: SemverIdent = unsafe { transmute(inline_bytes) };
    let ident_bits = unsafe { ptr::addr_of!(ident).cast::<NonZero<u64>>().read() };
    print_u64(ident_bits.get());
    unsafe {
        ptr::addr_of_mut!(ident)
            .cast::<u64>()
            .write(0x8877_6655_4433_2211u64);
    }
    print_u32(u32::from(ident.tail[0]));
    print_u32(u32::from(ident.tail[3]));

    // 4. Multi-level offset-0 layout-prefix pointer cast (`*mut Outer -> *mut u32`)
    let mut outer = black_box(Outer {
        inner: Inner {
            val: 100,
            extra: 200,
        },
        tag: 300,
    });
    unsafe {
        let p = ptr::addr_of_mut!(outer) as *mut u32;
        print_u32(*p);
        *p = 999;
    }
    print_u32(outer.inner.val);
    print_u32(outer.inner.extra);
    print_u32(outer.tag);

    // 5. Offset-0 array-prefix cast (`*mut ArrayPrefix -> *mut u32`)
    let mut ap = black_box(ArrayPrefix {
        arr: [11, 22, 33, 44],
        tag: 55,
    });
    unsafe {
        let p = ptr::addr_of_mut!(ap) as *mut u32;
        print_u32(*p.add(0));
        print_u32(*p.add(3));
        *p.add(2) = 777;
    }
    print_u32(ap.arr[2]);
    print_u32(ap.tag);
}
