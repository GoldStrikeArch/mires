//! Verifier control: the same dead written copy taken from memory that is not a constant.
#![no_std]
#![no_main]
#![allow(unused)]
#[path = "prelude.rs"]
mod prelude;
use prelude::*;
use core::hint::black_box;

#[derive(Clone, Copy)]
struct P { x: u32, y: u32 }
static ST: [P; 24] = [P { x: 100, y: 5100 }, P { x: 101, y: 5101 }, P { x: 102, y: 5102 }, P { x: 103, y: 5103 }, P { x: 104, y: 5104 }, P { x: 105, y: 5105 }, P { x: 106, y: 5106 }, P { x: 107, y: 5107 }, P { x: 108, y: 5108 }, P { x: 109, y: 5109 }, P { x: 110, y: 5110 }, P { x: 111, y: 5111 }, P { x: 112, y: 5112 }, P { x: 113, y: 5113 }, P { x: 114, y: 5114 }, P { x: 115, y: 5115 }, P { x: 116, y: 5116 }, P { x: 117, y: 5117 }, P { x: 118, y: 5118 }, P { x: 119, y: 5119 }, P { x: 120, y: 5120 }, P { x: 121, y: 5121 }, P { x: 122, y: 5122 }, P { x: 123, y: 5123 }];
#[inline(never)] fn through(s: &[P]) { let mut p = s[4]; p.x = 99; }
#[inline(never)] fn through_mut(s: &mut [P]) -> u32 { let mut p = s[2]; p.x = 77; s[2].x }
#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_str("static"); let mut p = ST[4]; p.x = 99; print_u32(ST[4].x);
    print_str("local array"); let local = [P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]; let mut q = local[1]; q.x = 99; print_u32(local[1].x);
    print_str("black-boxed local array"); let local2 = black_box([P { x: 1, y: 2 }, P { x: 3, y: 4 }, P { x: 5, y: 6 }]); let mut q = local2[1]; q.x = 99; print_u32(local2[1].x);
    print_str("parameter slice"); let arr = black_box([P { x: 10, y: 2 }; 8]); through(&arr); print_u32(arr[4].x);
    print_str("mutable parameter slice"); let mut arr2 = black_box([P { x: 20, y: 2 }; 8]); print_u32(through_mut(&mut arr2)); print_u32(arr2[2].x);
    print_str("static slice"); let s: &[P] = &ST; let mut r = s[5]; r.y = 1; print_u32(ST[5].y);
}
