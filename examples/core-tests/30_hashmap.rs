//! `mires_js::collections::{HashMap, HashSet}`: the host's own hash table, as Rust types.
//!
//! `std`'s `HashMap` is `hashbrown`, and `hashbrown` cannot be compiled by this backend: its group
//! scan reads sixteen control bytes as one wide integer and punnes the result back to a bitmask,
//! and its one allocation holds control bytes and entries at two element granularities. A heap
//! block here is a JavaScript array of ONE element type (`CONTRACT.md`, "Allocation"), so both are
//! refusals rather than slow paths. The table therefore comes from the engine, and a `HashMap`
//! **is** a host `Map`.
//!
//! What this file pins is the whole of `CONTRACT.md`, "Host collections":
//!
//! * every key class, including the ones that cross as **BigInt** (`u64`, `i64`, `u128`, `i128`)
//!   and the string class, where `&str`, `str` and `String` are interchangeable;
//! * an aggregate value mutated **through** `get_mut`, which only works because the reference
//!   names the place inside the host object rather than a copy of it;
//! * `insert` answering with the value the key held before, and `remove` answering with the value
//!   it is taking away;
//! * iteration over a snapshot of the keys, in the host's insertion order;
//! * `HashSet` over the same shim.

#![no_std]
#![no_main]

extern crate alloc;

#[path = "prelude.rs"]
mod prelude;
use prelude::*;

use alloc::string::{String, ToString};
use mires_js::collections::{HashMap, HashSet};

/// An aggregate value, so that `get_mut` has something with fields to write into.
struct Tally {
    hits: u32,
    last: i32,
}

/// Integer keys: plain JavaScript numbers, and the map is the only thing holding them.
fn integers() {
    let mut counts: HashMap<i32, i32> = HashMap::new();
    print_bool(counts.is_empty());

    counts.insert(1, 10);
    counts.insert(2, 20);
    counts.insert(3, 30);
    print_usize(counts.len());

    print_i32(*counts.get(&2).unwrap());
    print_bool(counts.get(&9).is_none());
    print_bool(counts.contains_key(&3));
    print_bool(counts.contains_key(&4));

    // Inserting over a key answers with what it held.
    print_i32(counts.insert(2, 22).unwrap());
    print_i32(*counts.get(&2).unwrap());
    // Inserting a new key answers with nothing.
    print_bool(counts.insert(4, 40).is_none());

    // Removing answers with what it took, and only once.
    print_i32(counts.remove(&1).unwrap());
    print_bool(counts.remove(&1).is_none());
    print_usize(counts.len());

    // A negative key and a key at the edge of the range are ordinary numbers.
    counts.insert(-7, -70);
    counts.insert(i32::MIN, 1);
    print_i32(*counts.get(&-7).unwrap());
    print_i32(*counts.get(&i32::MIN).unwrap());

    counts.clear();
    print_usize(counts.len());
    print_bool(counts.is_empty());
}

/// The widths above 53 bits cross as BigInt, and SameValueZero compares a BigInt by value.
fn wide_integers() {
    let mut big: HashMap<u64, i64> = HashMap::new();
    big.insert(1, 100);
    big.insert(u64::MAX, -1);
    big.insert(9_007_199_254_740_993, 7);

    print_i64(*big.get(&1).unwrap());
    print_i64(*big.get(&u64::MAX).unwrap());
    // A value one past 2^53, which a JavaScript number could not tell from its neighbour.
    print_i64(*big.get(&9_007_199_254_740_993).unwrap());
    print_bool(big.get(&9_007_199_254_740_992).is_none());
    print_usize(big.len());

    let mut wider: HashMap<u128, u32> = HashMap::new();
    wider.insert(u128::MAX, 5);
    print_u32(*wider.get(&u128::MAX).unwrap());
}

/// `bool` and `char` keys, the two classes that are neither a number nor a string in Rust but are
/// a boolean and a number to the host.
fn small_keys() {
    let mut flags: HashMap<bool, &str> = HashMap::new();
    flags.insert(true, "yes");
    flags.insert(false, "no");
    print_str(flags.get(&true).unwrap());
    print_str(flags.get(&false).unwrap());

    let mut letters: HashMap<char, u32> = HashMap::new();
    letters.insert('a', 1);
    letters.insert('z', 26);
    letters.insert('\u{1F980}', 999);
    print_u32(*letters.get(&'a').unwrap());
    print_u32(*letters.get(&'z').unwrap());
    print_u32(*letters.get(&'\u{1F980}').unwrap());
    print_bool(letters.get(&'b').is_none());
}

/// The string class: `&str` keys, `String` keys, and the two asking each other's questions.
fn strings() {
    let mut lengths: HashMap<&str, usize> = HashMap::new();
    lengths.insert("apple", 5);
    lengths.insert("fig", 3);
    print_usize(*lengths.get("apple").unwrap());
    print_bool(lengths.contains_key("fig"));
    print_bool(lengths.get("pear").is_none());

    // A `String` key is decoded to a host string on the way in, so the key the map holds outlives
    // the `String` that carried it and is found again by a plain `&str`.
    let mut owned: HashMap<String, u32> = HashMap::new();
    {
        let key = String::from("banana");
        owned.insert(key, 6);
    }
    print_u32(*owned.get("banana").unwrap());
    print_bool(owned.contains_key("banana"));

    // Two `String`s with the same text are one key.
    owned.insert("banana".to_string(), 7);
    print_usize(owned.len());
    print_u32(*owned.get("banana").unwrap());
    print_u32(owned.remove("banana").unwrap());
    print_usize(owned.len());

    // A `String` VALUE is an ordinary heap `Vec<u8>`, and the map holds it whole.
    let mut names: HashMap<u32, String> = HashMap::new();
    names.insert(1, String::from("ada"));
    names.insert(2, String::from("grace"));
    print_str(names.get(&1).unwrap());
    print_str(names.get(&2).unwrap());
    if let Some(name) = names.get_mut(&1) {
        name.push_str(" lovelace");
    }
    print_str(names.get(&1).unwrap());
    print_str(&names.remove(&2).unwrap());
}

/// An aggregate value written through `get_mut`: the reference names the place inside the host
/// object, so the write is what the next read sees.
fn aggregates() {
    let mut tallies: HashMap<&str, Tally> = HashMap::new();
    tallies.insert("a", Tally { hits: 0, last: 0 });
    tallies.insert("b", Tally { hits: 5, last: -1 });

    for _ in 0..3 {
        let entry = tallies.get_mut("a").unwrap();
        entry.hits += 1;
        entry.last = entry.hits as i32 * 10;
    }
    let a = tallies.get("a").unwrap();
    print_u32(a.hits);
    print_i32(a.last);

    // The old value comes back whole when a key is written over.
    let old = tallies.insert("b", Tally { hits: 1, last: 1 }).unwrap();
    print_u32(old.hits);
    print_i32(old.last);
    print_u32(tallies.get("b").unwrap().hits);

    // And when it is removed.
    let gone = tallies.remove("b").unwrap();
    print_u32(gone.hits);
    print_usize(tallies.len());

    // A tuple value, which is a JavaScript array rather than an object.
    let mut pairs: HashMap<u32, (i32, bool)> = HashMap::new();
    pairs.insert(1, (7, true));
    let pair = pairs.get(&1).unwrap();
    print_i32(pair.0);
    print_bool(pair.1);
}

/// Iteration, which walks a snapshot of the keys in the host's insertion order.
fn iteration() {
    let mut scores: HashMap<&str, i32> = HashMap::new();
    scores.insert("one", 1);
    scores.insert("two", 2);
    scores.insert("three", 3);

    let mut total = 0;
    for (key, value) in scores.iter() {
        print_str(key);
        total += *value;
    }
    print_i32(total);

    // `&map` iterates too, and the keys of an integer keyed map are integers.
    let mut squares: HashMap<u32, u32> = HashMap::new();
    for n in 1..5u32 {
        squares.insert(n, n * n);
    }
    let mut keys = 0;
    let mut values = 0;
    for (key, value) in &squares {
        keys += *key;
        values += *value;
    }
    print_u32(keys);
    print_u32(values);

    // Nothing to walk is not a special case.
    let empty: HashMap<i32, i32> = HashMap::new();
    print_usize(empty.iter().count());
}

/// `HashSet`, which is the same map with the unit for a value.
fn sets() {
    let mut seen: HashSet<&str> = HashSet::new();
    print_bool(seen.insert("a"));
    print_bool(seen.insert("b"));
    // The second one is not new.
    print_bool(seen.insert("a"));
    print_usize(seen.len());
    print_bool(seen.contains("a"));
    print_bool(seen.contains("c"));
    print_bool(seen.remove("a"));
    print_bool(seen.remove("a"));
    print_usize(seen.len());

    // Deduplicating a run of integers, which is what a set is for.
    let mut numbers: HashSet<u32> = HashSet::new();
    // By reference: an array iterated BY VALUE is a separate gap in the emitter, and this file is
    // about collections.
    for n in [3u32, 1, 4, 1, 5, 9, 2, 6, 5, 3].iter() {
        numbers.insert(*n);
    }
    print_usize(numbers.len());
    let mut sum = 0;
    for n in &numbers {
        sum += *n;
    }
    print_u32(sum);

    numbers.clear();
    print_bool(numbers.is_empty());
}

/// Two maps are two host objects, and a map moved is the same host object.
fn identity() {
    let mut first: HashMap<i32, i32> = HashMap::new();
    let mut second: HashMap<i32, i32> = HashMap::new();
    first.insert(1, 1);
    second.insert(1, 2);
    print_i32(*first.get(&1).unwrap());
    print_i32(*second.get(&1).unwrap());

    // Moving a map into a function and back keeps the entries: the value IS the host `Map`, so
    // there is nothing to copy and nothing to lose.
    let grown = grow(first);
    print_usize(grown.len());
    print_i32(*grown.get(&2).unwrap());
}

fn grow(mut map: HashMap<i32, i32>) -> HashMap<i32, i32> {
    map.insert(2, 2);
    map
}

#[unsafe(no_mangle)]
pub fn rust_entry() {
    integers();
    wide_integers();
    small_keys();
    strings();
    aggregates();
    iteration();
    sets();
    identity();
}
