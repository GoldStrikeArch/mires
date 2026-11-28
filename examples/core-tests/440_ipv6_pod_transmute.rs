//! Layout-driven POD transmute for `core::net::Ipv6Addr` (`[u8; 16] <-> [u16; 8]`) and general
//! equal-size POD layouts (nested arrays, structs, tuples).

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::format;
use core::hint::black_box;
use core::mem::transmute;
use core::net::Ipv6Addr;

#[repr(C)]
#[derive(Clone, Copy)]
struct PairU16 {
    lo: u16,
    hi: u16,
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_entry() {
    let ip: Ipv6Addr = black_box("2001:db8::1").parse().unwrap();
    print_str(&format!("{ip}"));
    let segs = ip.segments();
    print_u32(u32::from(segs[0]));
    print_u32(u32::from(segs[1]));
    print_u32(u32::from(segs[7]));

    let octets = ip.octets();
    print_u32(u32::from(octets[0]));
    print_u32(u32::from(octets[1]));
    print_u32(u32::from(octets[15]));

    let rebuilt = Ipv6Addr::from(black_box(segs));
    print_bool(rebuilt == ip);

    let local: Ipv6Addr = black_box("fe80::1ff:fe23:4567:890a").parse().unwrap();
    print_str(&format!("{local}"));
    let lsegs = local.segments();
    print_u32(u32::from(lsegs[4]));
    print_u32(u32::from(lsegs[7]));

    // Direct POD transmutes: `[u8; 16] <-> [u16; 8]`, `[[u8; 4]; 4] <-> [u32; 4]`, `PairU16 <-> u32`.
    let raw16: [u8; 16] = black_box([
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
        0x10,
    ]);
    let words16: [u16; 8] = unsafe { transmute(raw16) };
    print_u32(u32::from(words16[0]));
    print_u32(u32::from(words16[7]));
    let back16: [u8; 16] = unsafe { transmute(black_box(words16)) };
    print_bool(back16 == raw16);

    let nested: [[u8; 4]; 4] = unsafe { transmute(black_box(raw16)) };
    let words32: [u32; 4] = unsafe { transmute(black_box(nested)) };
    print_u32(words32[0]);
    print_u32(words32[3]);

    let p = black_box(PairU16 {
        lo: 0x1234,
        hi: 0xabcd,
    });
    let packed: u32 = unsafe { transmute(p) };
    print_u32(packed);
    let unpacked: PairU16 = unsafe { transmute(black_box(packed)) };
    print_u32(u32::from(unpacked.lo));
    print_u32(u32::from(unpacked.hi));
}
