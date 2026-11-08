//! Recursive product payloads and the first bounded ADT/iterator boundary matrix.
//!
//! The profitable paths keep nested products and tags in scalar lanes across private direct
//! calls. The mutation, indirect, vtable, exported, custom-drop, and iterator-method paths are
//! equally important negative controls: they must keep their canonical Rust value model until a
//! later storage or callable plan can prove the stronger rewrite.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
pub struct Pair {
    left: u32,
    right: u32,
}

#[derive(Clone, Copy)]
struct Nested {
    pair: Pair,
    bias: u32,
}

#[inline(never)]
fn make_nested_option(seed: u32) -> Option<Nested> {
    if seed & 1 == 0 {
        Some(Nested {
            pair: Pair {
                left: seed + 1,
                right: seed + 2,
            },
            bias: seed + 3,
        })
    } else {
        None
    }
}

#[inline(never)]
fn nested_option_helper_one(value: Option<Nested>) -> u32 {
    nested_option_helper_two(value)
}

#[inline(never)]
fn nested_option_helper_two(value: Option<Nested>) -> u32 {
    match value {
        Some(value) => value
            .pair
            .left
            .wrapping_add(value.pair.right.wrapping_mul(2))
            .wrapping_add(value.bias.wrapping_mul(3)),
        None => 13,
    }
}

#[inline(never)]
fn nested_option_path_a(seed: u32) -> u32 {
    nested_option_helper_one(make_nested_option(seed))
}

#[inline(never)]
fn nested_option_path_b(seed: u32) -> u32 {
    nested_option_helper_one(make_nested_option(seed))
}

#[inline(never)]
fn hot_nested_option_loop(rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(nested_option_helper_one(make_nested_option(index)));
        index += 1;
    }
    total
}

#[inline(never)]
fn make_option(seed: u32) -> Option<u32> {
    if seed & 1 == 0 { Some(seed + 5) } else { None }
}

#[inline(never)]
fn option_helper_one(value: Option<u32>) -> u32 {
    option_helper_two(value)
}

#[inline(never)]
fn option_helper_two(value: Option<u32>) -> u32 {
    match value {
        Some(value) => value.wrapping_mul(3),
        None => 7,
    }
}

#[inline(never)]
fn option_path_a(seed: u32) -> u32 {
    option_helper_one(make_option(seed))
}

#[inline(never)]
fn option_path_b(seed: u32) -> u32 {
    option_helper_one(make_option(seed))
}

#[inline(never)]
fn hot_option_loop(rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(option_helper_one(make_option(index)));
        index += 1;
    }
    total
}

#[inline(never)]
fn make_pair_option(seed: u32) -> Option<Pair> {
    if seed % 3 == 0 {
        Some(Pair {
            left: seed + 1,
            right: seed.wrapping_mul(2),
        })
    } else {
        None
    }
}

#[inline(never)]
fn pair_option_helper_one(value: Option<Pair>) -> u32 {
    pair_option_helper_two(value)
}

#[inline(never)]
fn pair_option_helper_two(value: Option<Pair>) -> u32 {
    match value {
        Some(pair) => pair.left.wrapping_mul(5).wrapping_add(pair.right),
        None => 11,
    }
}

#[inline(never)]
fn pair_option_path_a(seed: u32) -> u32 {
    pair_option_helper_one(make_pair_option(seed))
}

#[inline(never)]
fn pair_option_path_b(seed: u32) -> u32 {
    pair_option_helper_one(make_pair_option(seed))
}

#[inline(never)]
fn hot_pair_option_loop(rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(pair_option_helper_one(make_pair_option(index)));
        index += 1;
    }
    total
}

#[derive(Clone, Copy)]
enum PairChoice {
    Left { left: Pair },
    Right,
}

#[inline(never)]
fn make_pair_choice(seed: u32) -> PairChoice {
    if seed & 1 == 0 {
        PairChoice::Left {
            left: Pair {
                left: seed + 2,
                right: seed.wrapping_mul(3),
            },
        }
    } else {
        PairChoice::Right
    }
}

#[inline(never)]
fn pair_choice_helper_one(value: PairChoice) -> u32 {
    pair_choice_helper_two(value)
}

#[inline(never)]
fn pair_choice_helper_two(value: PairChoice) -> u32 {
    match value {
        PairChoice::Left { left } => left.left.wrapping_mul(7).wrapping_add(left.right),
        PairChoice::Right => 17,
    }
}

#[inline(never)]
fn pair_choice_path_a(seed: u32) -> u32 {
    pair_choice_helper_one(make_pair_choice(seed))
}

#[inline(never)]
fn pair_choice_path_b(seed: u32) -> u32 {
    pair_choice_helper_one(make_pair_choice(seed))
}

#[inline(never)]
fn hot_pair_choice_loop(rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(pair_choice_helper_one(make_pair_choice(index)));
        index += 1;
    }
    total
}

#[inline(never)]
fn make_nested(seed: u32) -> Nested {
    Nested {
        pair: Pair {
            left: seed + 1,
            right: seed.wrapping_mul(2),
        },
        bias: seed.wrapping_mul(5).wrapping_sub(3),
    }
}

#[inline(never)]
fn nested_helper_one(value: Nested) -> u32 {
    nested_helper_two(value)
}

#[inline(never)]
fn nested_helper_two(value: Nested) -> u32 {
    value
        .pair
        .left
        .wrapping_add(value.pair.right.wrapping_mul(3))
        .wrapping_add(value.bias.wrapping_mul(5))
}

#[inline(never)]
fn nested_path_a(seed: u32) -> u32 {
    nested_helper_one(make_nested(seed))
}

#[inline(never)]
fn nested_path_b(seed: u32) -> u32 {
    nested_helper_one(make_nested(seed))
}

#[inline(never)]
fn hot_nested_loop(rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(nested_helper_one(make_nested(index + 1)));
        index += 1;
    }
    total
}

#[inline(never)]
fn iterator_control_flow(limit: u32, stop: u32) -> core::ops::ControlFlow<u32, u32> {
    (0..limit).try_fold(0u32, |acc, value| {
        if value == stop {
            core::ops::ControlFlow::Break(acc.wrapping_add(value))
        } else {
            core::ops::ControlFlow::Continue(acc.wrapping_add(value))
        }
    })
}

#[inline(never)]
fn consume_control_flow(value: core::ops::ControlFlow<u32, u32>) -> u32 {
    match value {
        core::ops::ControlFlow::Break(value) => value.wrapping_mul(2),
        core::ops::ControlFlow::Continue(value) => value.wrapping_add(1_000),
    }
}

#[inline(never)]
fn control_flow_path_a(limit: u32, stop: u32) -> u32 {
    consume_control_flow(iterator_control_flow(limit, stop))
}

#[inline(never)]
fn control_flow_path_b(limit: u32, stop: u32) -> u32 {
    consume_control_flow(iterator_control_flow(limit, stop))
}

#[inline(never)]
fn hot_control_flow_loop(rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(consume_control_flow(iterator_control_flow(8, index)));
        index += 1;
    }
    total
}

#[inline(never)]
fn make_chunks(bytes: &[u8]) -> core::slice::ChunksExact<'_, u8> {
    bytes.chunks_exact(16)
}

#[inline(never)]
fn consume_chunks(mut chunks: core::slice::ChunksExact<'_, u8>) -> u32 {
    let mut total = 0u32;
    while let Some(chunk) = chunks.next() {
        total = total
            .wrapping_add(chunk[0] as u32)
            .wrapping_add(chunk[15] as u32)
            .wrapping_add(chunk.len() as u32);
    }
    total.wrapping_add(chunks.remainder().len() as u32)
}

#[inline(never)]
fn chunks_path_a(bytes: &[u8]) -> u32 {
    consume_chunks(make_chunks(bytes))
}

#[inline(never)]
fn chunks_path_b(bytes: &[u8]) -> u32 {
    consume_chunks(make_chunks(bytes))
}

#[inline(never)]
fn hot_chunks_loop(bytes: &[u8], rounds: u32) -> u32 {
    let mut total = 0u32;
    let mut index = 0;
    while index < rounds {
        total = total.wrapping_add(consume_chunks(make_chunks(bytes)));
        index += 1;
    }
    total
}

#[inline(never)]
fn mutate_tagged(mut value: Option<Pair>, seed: u32) -> u32 {
    if seed & 1 == 0 {
        value = Some(Pair {
            left: seed,
            right: seed + 1,
        });
    }
    pair_option_helper_two(value)
}

#[inline(never)]
fn borrow_tagged(value: &Option<Pair>) -> u32 {
    match value {
        Some(pair) => pair.left ^ pair.right,
        None => 0,
    }
}

#[inline(never)]
fn indirect_tagged(seed: u32, callback: fn(u32) -> Option<Pair>) -> u32 {
    pair_option_helper_two(callback(seed))
}

trait Score {
    fn score(&self) -> u32;
}

impl Score for Pair {
    fn score(&self) -> u32 {
        self.left.wrapping_mul(9).wrapping_add(self.right)
    }
}

#[inline(never)]
fn vtable_score(value: &dyn Score) -> u32 {
    value.score()
}

#[inline(never)]
fn local_closure(seed: u32) -> u32 {
    let bias = seed.wrapping_mul(3).wrapping_sub(3);
    let mix = |value: u32| value.wrapping_mul(seed).wrapping_add(bias);
    mix(2).wrapping_add(mix(3))
}

#[inline(never)]
fn escaping_closure(seed: u32) -> impl Fn(u32) -> u32 {
    move |value| value.wrapping_add(seed)
}

struct Trace(u32);

impl Drop for Trace {
    fn drop(&mut self) {
        print_u32(self.0);
    }
}

#[inline(never)]
fn drop_early(flag: bool) -> u32 {
    let _trace = Trace(700 + flag as u32);
    if flag { 1 } else { 2 }
}

/// The exported body and its panic arm retain the canonical tagged ABI.
#[inline(never)]
#[unsafe(no_mangle)]
pub fn exported_or_panic(value: Option<Pair>) -> u32 {
    match value {
        Some(pair) => pair.left.wrapping_add(pair.right),
        None => panic!("phase 6C tagged boundary"),
    }
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(option_path_a(4));
    print_u32(option_path_b(5));
    print_u32(pair_option_path_a(6));
    print_u32(pair_option_path_b(7));
    print_u32(pair_choice_path_a(4));
    print_u32(pair_choice_path_b(5));
    print_u32(nested_option_path_a(2));
    print_u32(nested_option_path_b(3));
    print_u32(nested_path_a(3));
    print_u32(nested_path_b(8));
    print_u32(control_flow_path_a(8, 4));
    print_u32(control_flow_path_b(5, 99));
    print_u32(hot_option_loop(8));
    print_u32(hot_pair_option_loop(9));
    print_u32(hot_pair_choice_loop(6));
    print_u32(hot_nested_option_loop(6));
    print_u32(hot_nested_loop(4));
    print_u32(hot_control_flow_loop(6));

    let bytes = [
        1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35,
    ];
    print_u32(chunks_path_a(&bytes));
    print_u32(chunks_path_b(&bytes[..32]));
    print_u32(hot_chunks_loop(&bytes, 3));

    print_u32(mutate_tagged(None, 4));
    let borrowed = Some(Pair { left: 10, right: 12 });
    print_u32(borrow_tagged(&borrowed));
    print_u32(indirect_tagged(6, make_pair_option));
    print_u32(vtable_score(&Pair { left: 3, right: 4 }));
    print_u32(local_closure(5));
    let escaped = escaping_closure(9);
    print_u32(escaped(4));
    print_u32(drop_early(true));
    print_u32(drop_early(false));
}
