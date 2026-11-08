//! Products stay scalar through loop merges, direct-call chains, and parameter SCCs.

#![no_std]
#![no_main]

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

#[derive(Clone, Copy)]
struct Pair {
    left: u32,
    right: u32,
}

#[inline(never)]
fn make_forward_pair(seed: u32) -> Pair {
    Pair {
        left: seed.wrapping_add(1),
        right: seed.wrapping_mul(3),
    }
}

#[inline(never)]
fn consume_forward_pair(pair: Pair) -> u32 {
    pair.left
        .wrapping_mul(7)
        .wrapping_add(pair.right.wrapping_mul(11))
}

#[inline(never)]
fn forwarded_result(seed: u32) -> u32 {
    consume_forward_pair(make_forward_pair(seed))
}

#[inline(never)]
fn loop_product(seed: u32, rounds: u32) -> Pair {
    let mut pair = Pair {
        left: seed,
        right: seed.wrapping_add(1),
    };
    let mut i = 0;
    while i < rounds {
        pair = Pair {
            left: pair.right.wrapping_add(1),
            right: pair.left.wrapping_add(pair.right),
        };
        i += 1;
    }
    pair
}

#[inline(never)]
fn bounce_even(pair: Pair, depth: u32) -> u32 {
    if depth == 0 {
        pair.left ^ pair.right
    } else {
        bounce_odd(
            Pair {
                left: pair.left.wrapping_add(1),
                right: pair.right.wrapping_add(2),
            },
            depth - 1,
        )
    }
}

#[inline(never)]
fn bounce_odd(pair: Pair, depth: u32) -> u32 {
    if depth == 0 {
        pair.left.wrapping_add(pair.right)
    } else {
        bounce_even(
            Pair {
                left: pair.left.wrapping_mul(3),
                right: pair.right.wrapping_add(1),
            },
            depth - 1,
        )
    }
}

#[inline(never)]
fn tagged_option(rounds: u32) -> u32 {
    let mut state: Option<u32> = None;
    let mut total = match state {
        Some(value) => value,
        None => 0,
    };
    let mut i = 0;
    while i < rounds {
        state = if i & 1 == 0 { Some(i + 3) } else { None };
        total = total.wrapping_add(match state {
            Some(value) => value,
            None => 1,
        });
        i += 1;
    }
    total
}

#[inline(never)]
fn tagged_result(rounds: u32) -> u32 {
    let mut state: Result<u32, u32> = Ok(0);
    let mut total = match state {
        Ok(value) => value,
        Err(error) => error.wrapping_mul(2),
    };
    let mut i = 0;
    while i < rounds {
        state = if i % 3 == 0 {
            Err(i + 10)
        } else {
            Ok(i + 2)
        };
        total = total.wrapping_add(match state {
            Ok(value) => value,
            Err(error) => error.wrapping_mul(2),
        });
        i += 1;
    }
    total
}

#[derive(Clone, Copy)]
enum Choice {
    Left { left: u32 },
    Right { right: u32 },
}

#[inline(never)]
fn make_option(seed: u32) -> Option<u32> {
    if seed & 1 == 0 {
        Some(seed + 7)
    } else {
        None
    }
}

#[inline(never)]
fn consume_option(value: Option<u32>) -> u32 {
    match value {
        Some(value) => value.wrapping_mul(3),
        None => 5,
    }
}

#[inline(never)]
fn option_once_a(seed: u32) -> u32 {
    consume_option(make_option(seed))
}

#[inline(never)]
fn option_once_b(seed: u32) -> u32 {
    consume_option(make_option(seed))
}

#[inline(never)]
fn make_result(seed: u32) -> Result<u32, u32> {
    if seed % 3 == 0 {
        Err(seed + 2)
    } else {
        Ok(seed + 1)
    }
}

#[inline(never)]
fn consume_result(value: Result<u32, u32>) -> u32 {
    match value {
        Ok(value) => value.wrapping_mul(2),
        Err(error) => error.wrapping_mul(7),
    }
}

#[inline(never)]
fn result_once_a(seed: u32) -> u32 {
    consume_result(make_result(seed))
}

#[inline(never)]
fn result_once_b(seed: u32) -> u32 {
    consume_result(make_result(seed))
}

#[inline(never)]
fn make_choice(seed: u32) -> Choice {
    if seed & 1 == 0 {
        Choice::Left { left: seed + 10 }
    } else {
        Choice::Right { right: seed + 20 }
    }
}

#[inline(never)]
fn consume_choice(value: Choice) -> u32 {
    match value {
        Choice::Left { left } => left + 1,
        Choice::Right { right } => right + 2,
    }
}

#[inline(never)]
fn choice_once_a(seed: u32) -> u32 {
    consume_choice(make_choice(seed))
}

#[inline(never)]
fn choice_once_b(seed: u32) -> u32 {
    consume_choice(make_choice(seed))
}

/// Outside callers retain the canonical enum object. Prepared direct callers target one private
/// `tag, payload` body.
#[inline(never)]
#[unsafe(no_mangle)]
pub fn exported_tagged(seed: u32) -> Option<u32> {
    if seed & 1 == 0 {
        Some(seed + 7)
    } else {
        None
    }
}

#[inline(never)]
fn direct_exported_tagged_a(seed: u32) -> u32 {
    consume_option(exported_tagged(seed))
}

#[inline(never)]
fn direct_exported_tagged_b(seed: u32) -> u32 {
    consume_option(exported_tagged(seed))
}

#[inline(never)]
fn indirect_tagged(seed: u32, callback: fn(u32) -> Option<u32>) -> u32 {
    consume_option(callback(seed))
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    print_u32(forwarded_result(5));
    let looped = loop_product(2, 4);
    print_u32(looped.left.wrapping_add(looped.right));
    print_u32(bounce_even(Pair { left: 1, right: 2 }, 5));
    print_u32(tagged_option(5));
    print_u32(tagged_result(5));
    print_u32(option_once_a(4));
    print_u32(option_once_b(5));
    print_u32(result_once_a(4));
    print_u32(result_once_b(3));
    print_u32(choice_once_a(2));
    print_u32(choice_once_b(3));
    print_u32(direct_exported_tagged_a(6) + direct_exported_tagged_b(7));
    print_u32(indirect_tagged(5, exported_tagged));
}
